from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
SCRIPT_ROOT = SKILL_ROOT / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.contracts.plan import render_plan_contract, validate_plan_contract
from worklib.skills.selection import selection_sha256
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.hierarchy.selection import build_hierarchy_selection


class PlanInstructionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.project_root = Path(self.temporary_directory.name).resolve()
        hierarchy_selection = build_hierarchy_selection(
            {"decision": "general_only", "selections": []},
            skill_root=SKILL_ROOT,
        )
        self.contract: dict[str, object] = {
            "schema": "work-plan/v1",
            "requirement_id": "example",
            "status": "confirmed",
            "title": "Example Plan",
            "summary": "需求摘要",
            "artifacts": {
                "plan": "outputs/work/plans/example.md",
                "task": "outputs/work/tasks/example.md",
                "execution": "outputs/work/executions/example",
            },
            "hierarchy_selection": hierarchy_selection,
            "work_instruction_selection": build_work_instruction_selection(
                skill_root=SKILL_ROOT,
                mode="plan",
                selected_paths=hierarchy_selection["selected_paths"],
            ),
            "skill_selection": {
                "schema": "work-skill-selection/v1",
                "decision": "base_only",
                "skills": [],
                "selection_sha256": selection_sha256("base_only", []),
            },
            "goals": [{"id": "GOAL-001", "statement": "完成目標"}],
            "scope": [
                {
                    "id": "SCOPE-001",
                    "kind": "in_scope",
                    "statement": "處理範圍",
                    "goal_ids": ["GOAL-001"],
                }
            ],
            "deliverables": [
                {
                    "id": "DELIVERABLE-001",
                    "statement": "交付成果",
                    "goal_ids": ["GOAL-001"],
                    "acceptance_ids": ["ACCEPTANCE-001"],
                }
            ],
            "acceptance_criteria": [
                {
                    "id": "ACCEPTANCE-001",
                    "statement": "可觀察結果",
                    "deliverable_ids": ["DELIVERABLE-001"],
                }
            ],
        }

    def _validate(self, contract: dict[str, object] | None = None) -> dict[str, object]:
        value = contract or self.contract
        return validate_plan_contract(
            render_plan_contract(value),
            source="test",
            actual_plan_path="outputs/work/plans/example.md",
            project_root=self.project_root,
            user_config_root=str(self.project_root),
        )

    def test_valid_plan_returns_instruction_fingerprint(self) -> None:
        result = self._validate()

        self.assertEqual(result["schema"], "work-plan-validation/v1")
        self.assertEqual(result["item_count"], 4)
        self.assertEqual(
            result["hierarchy_selection_sha256"],
            self.contract["hierarchy_selection"]["selection_sha256"],  # type: ignore[index]
        )
        self.assertEqual(
            result["work_instructions_sha256"],
            self.contract["work_instruction_selection"]["instructions_sha256"],  # type: ignore[index]
        )
        self.assertNotIn("rules_sha256", result)

    def test_render_uses_canonical_instruction_field_order(self) -> None:
        raw = render_plan_contract(dict(reversed(self.contract.items())))
        payload = json.loads(raw.decode("utf-8").split("```json\n", 1)[1].rsplit("\n```", 1)[0])

        self.assertEqual(list(payload)[6], "hierarchy_selection")
        self.assertEqual(list(payload)[7], "work_instruction_selection")
        self.assertEqual(list(payload)[8], "skill_selection")
        self.assertEqual(
            list(payload["work_instruction_selection"]),
            [
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256",
            ],
        )
        self.assertEqual(
            list(payload["work_instruction_selection"]["sources"][0]),
            ["kind", "logical_name", "canonical_sha256"],
        )

    def test_legacy_rule_selection_is_rejected(self) -> None:
        contract = copy.deepcopy(self.contract)
        contract["rule_selection"] = contract.pop("work_instruction_selection")

        with self.assertRaises(WorkError) as context:
            self._validate(contract)
        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["work_instruction_selection"])
        self.assertEqual(context.exception.details["unknown"], ["rule_selection"])

    def test_legacy_source_layer_is_rejected(self) -> None:
        contract = copy.deepcopy(self.contract)
        selection = contract["work_instruction_selection"]
        assert isinstance(selection, dict)
        sources = selection["sources"]
        assert isinstance(sources, list) and isinstance(sources[0], dict)
        sources[0]["layer"] = "project"

        with self.assertRaises(WorkError) as context:
            self._validate(contract)
        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["unknown"], ["layer"])

    def test_stale_instruction_fingerprint_is_rejected(self) -> None:
        contract = copy.deepcopy(self.contract)
        selection = contract["work_instruction_selection"]
        assert isinstance(selection, dict)
        selection["instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            self._validate(contract)
        self.assertEqual(context.exception.code, "work_instructions_fingerprint_mismatch")

    def test_instruction_source_order_mismatch_is_rejected(self) -> None:
        contract = copy.deepcopy(self.contract)
        selection = contract["work_instruction_selection"]
        assert isinstance(selection, dict)
        sources = selection["sources"]
        assert isinstance(sources, list)
        sources[0], sources[1] = sources[1], sources[0]

        with self.assertRaises(WorkError) as context:
            self._validate(contract)
        self.assertEqual(
            context.exception.code,
            "work_instruction_selection_sources_mismatch",
        )


if __name__ == "__main__":
    unittest.main()
