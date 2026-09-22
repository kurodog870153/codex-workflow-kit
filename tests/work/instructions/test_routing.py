from __future__ import annotations

import sys
import shutil
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.models.common.errors import WorkError
from worklib.services.workflow.routing import (
    BOOTSTRAP,
    DECISION_TABLE,
    EVENT_SOURCES,
    FORMAL_EVENTS,
    SOURCE_CATALOG,
    build_routing_selection,
)


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"


class InstructionRoutingTests(unittest.TestCase):
    def test_plan_operation_selects_only_bootstrap_and_plan_entry(self) -> None:
        result = build_routing_selection(
            SKILL_ROOT, status="plan_required", next_action="prepare_plan", confirmation=True,
        )
        self.assertEqual(result["routing_status"], "VALID")
        self.assertEqual(result["required_instruction_sources"], [
            "work.instruction-loading",
            "work.shared.invocation",
            "work.shared.skill-discovery",
            "work.shared.skill-selection",
            "work.shared.source-loading",
            "work.shared.artifact-paths",
            "work.shared.fingerprints",
            "work.workflow.plan",
            "work.workflow.plan.apply-confirmed-skills",
            "work.workflow.plan.complete-the-request",
            "work.workflow.plan.use-the-deterministic-plan-contract",
        ])
        self.assertNotIn("work.workflow.task", result["required_instruction_sources"])
        self.assertNotIn("work.workflow.execute", result["required_instruction_sources"])

    def test_unknown_operation_requires_review_without_full_fallback(self) -> None:
        result = build_routing_selection(
            SKILL_ROOT, status="unknown", next_action="unknown_action", confirmation=True,
        )
        self.assertEqual(result["routing_status"], "REVIEW_REQUIRED")
        self.assertEqual(result["required_instruction_sources"], ["work.instruction-loading"])

    def test_every_catalog_source_is_reachable_from_a_canonical_input(self) -> None:
        reached = set()
        for next_action in DECISION_TABLE:
            result = build_routing_selection(
                SKILL_ROOT, status="verified", next_action=next_action,
                confirmation=True,
            )
            self.assertEqual(result["routing_status"], "VALID", next_action)
            reached.update(result["required_instruction_sources"])
        scenarios = []
        scenarios.extend(("progress", (event,)) for event in sorted({
            "progress_read", "progress_save",
        }))
        scenarios.extend(("execute", (event,)) for event in sorted({
            "invocation", "handoff", "migration", "revision", "reconciliation",
            "recovery", "correction", "command_correction", "command_execution",
            "attempt_start", "attempt_close", "delegation", "invalid_artifact",
            "skill_load", "safety_rejection", "file_failure",
        }))
        for mode, events in scenarios:
            result = build_routing_selection(
                SKILL_ROOT, status="verified", next_action="continue_execution",
                confirmation=False, mode=mode, formal_events=events,
                role="worker" if events == ("delegation",) else "main",
            )
            self.assertEqual(result["routing_status"], "VALID", events)
            reached.update(result["required_instruction_sources"])
        self.assertEqual(reached, set(SOURCE_CATALOG))

    def test_event_role_and_authorization_are_fingerprint_bound(self) -> None:
        base = build_routing_selection(
            SKILL_ROOT, status="verified", next_action="continue_execution",
            confirmation=False,
        )
        delegated = build_routing_selection(
            SKILL_ROOT, status="verified", next_action="continue_execution",
            confirmation=False, formal_events=("delegation",), role="worker",
            authorization_state="authorized",
        )
        self.assertNotEqual(base["selection_sha256"], delegated["selection_sha256"])
        self.assertIn("work.shared.internal-envelope", delegated["required_instruction_sources"])

    def test_each_formal_event_adds_only_its_declared_sources(self) -> None:
        base = build_routing_selection(
            SKILL_ROOT, status="verified", next_action="continue_execution",
            confirmation=False,
        )
        base_sources = set(base["required_instruction_sources"])
        for event in sorted(FORMAL_EVENTS):
            with self.subTest(event=event):
                role = "worker" if event == "delegation" else "main"
                routed = build_routing_selection(
                    SKILL_ROOT, status="verified", next_action="continue_execution",
                    confirmation=False, formal_events=(event,), role=role,
                )
                selected = set(routed["required_instruction_sources"])
                self.assertEqual(routed["routing_status"], "VALID")
                self.assertEqual(selected - base_sources, set(EVENT_SOURCES[event]) - base_sources)
                unrelated = next(
                    candidate for candidate in FORMAL_EVENTS
                    if candidate != event and set(EVENT_SOURCES[candidate]) - base_sources - set(EVENT_SOURCES[event])
                )
                self.assertTrue((set(EVENT_SOURCES[unrelated]) - base_sources) - selected)

    def test_conflicting_or_unverified_routing_input_requires_review(self) -> None:
        for overrides in (
            {"formal_events": ("handoff", "recovery")},
            {"formal_events": ("unknown",)},
            {"role": "worker"},
            {"mode": "unknown"},
        ):
            with self.subTest(overrides=overrides):
                result = build_routing_selection(
                    SKILL_ROOT, status="verified", next_action="continue_execution",
                    confirmation=False, **overrides,
                )
                self.assertEqual(result["routing_status"], "REVIEW_REQUIRED")
                self.assertEqual(result["required_instruction_sources"], [BOOTSTRAP])

    def test_every_operation_is_deterministic_and_selects_existing_sources(self) -> None:
        for next_action, expected in DECISION_TABLE.items():
            with self.subTest(next_action=next_action):
                first = build_routing_selection(
                    SKILL_ROOT, status="verified", next_action=next_action, confirmation=True,
                )
                second = build_routing_selection(
                    SKILL_ROOT, status="verified", next_action=next_action, confirmation=True,
                )
                self.assertEqual(first, second)
                self.assertEqual(first["routing_status"], "VALID")
                self.assertEqual(set(first["required_instruction_sources"]), set(expected))
                for logical_name in first["required_instruction_sources"]:
                    self.assertTrue((SKILL_ROOT / SOURCE_CATALOG[logical_name][0]).is_file())

    def test_each_operation_excludes_unrelated_workflow_entries(self) -> None:
        workflow_entries = {
            name for name in SOURCE_CATALOG
            if name.startswith("work.workflow.") and name.count(".") == 2
        }
        for next_action in DECISION_TABLE:
            result = build_routing_selection(
                SKILL_ROOT, status="verified", next_action=next_action, confirmation=True,
            )
            selected = set(result["required_instruction_sources"])
            self.assertLess(len(selected & workflow_entries), len(workflow_entries))

    def test_missing_source_is_a_hard_stop(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "work"
            shutil.copytree(SKILL_ROOT, root)
            (root / SOURCE_CATALOG["work.shared.invocation"][0]).unlink()
            with self.assertRaises(WorkError) as caught:
                build_routing_selection(
                    root, status="plan_required", next_action="prepare_plan", confirmation=True,
                )
        self.assertEqual(caught.exception.code, "routed_instruction_source_missing")

    def test_selected_source_drift_changes_only_selection_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "work"
            shutil.copytree(SKILL_ROOT, root)
            before = build_routing_selection(
                root, status="plan_required", next_action="prepare_plan", confirmation=True,
            )
            unrelated = root / SOURCE_CATALOG["work.workflow.execute"][0]
            unrelated.write_text(unrelated.read_text(encoding="utf-8") + "\nUnrelated.\n", encoding="utf-8")
            unchanged = build_routing_selection(
                root, status="plan_required", next_action="prepare_plan", confirmation=True,
            )
            self.assertEqual(before["selection_sha256"], unchanged["selection_sha256"])
            selected = root / SOURCE_CATALOG["work.shared.invocation"][0]
            selected.write_text(selected.read_text(encoding="utf-8") + "\nSelected.\n", encoding="utf-8")
            changed = build_routing_selection(
                root, status="plan_required", next_action="prepare_plan", confirmation=True,
            )
            self.assertNotEqual(before["selection_sha256"], changed["selection_sha256"])


if __name__ == "__main__":
    unittest.main()
