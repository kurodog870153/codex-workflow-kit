from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
if str(SCRIPT_ROOT) not in sys.path:
    sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.contracts.handoff import validate_handoff_contract


class HandoffInstructionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.project_temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.project_temp.cleanup)
        self.project_root = Path(self.project_temp.name).resolve()

    def target(self, stage: str) -> dict[str, object]:
        return {"stage": stage}

    def handoff(self, direction: str) -> dict[str, object]:
        source_stage, target_stage = {
            "plan_to_task": ("plan", "task"),
            "task_to_execute": ("task", "execute"),
            "execute_to_task": ("execute", "task"),
            "task_to_plan": ("task", "plan"),
            "execute_to_plan": ("execute", "plan"),
        }[direction]

        if direction == "plan_to_task":
            source: dict[str, object] = {
                "stage": source_stage,
                "plan_sha256": "a" * 64,
                "skill_selection_sha256": "d" * 64,
            }
        elif direction == "task_to_plan":
            source = {
                "stage": source_stage,
                "plan_sha256": "a" * 64,
                "task_spec_id": "TASK-SPEC-001",
                "task_id": "TASK-001",
                "skill_selection_sha256": "d" * 64,
                "skill_id": None,
            }
        else:
            source = {
                "stage": source_stage,
                "task_spec_id": "TASK-SPEC-001",
                "task_id": "TASK-001",
                "task_sha256": "b" * 64,
                "task_instructions_sha256": "c" * 64,
                "skill_id": None,
            }
            if direction.startswith("execute_to_"):
                source["execute_skill_selection_sha256"] = "d" * 64
                source["execution_context"] = {
                    "attempt": {"status": "not_created"},
                    "phase": "preflight",
                    "issue_type": "specification_defect",
                    "reason": "The specification needs clarification.",
                }
            else:
                source["skill_selection_sha256"] = "d" * 64

        contract: dict[str, object] = {
            "schema": "work-handoff/v1",
            "marker": "WORK-HANDOFF",
            "direction": direction,
            "requirement_id": "example",
            "artifacts": {
                "plan": "outputs/work/plans/example.md",
                "task": "outputs/work/tasks/example.md",
                "execution": "outputs/work/executions/example",
            },
            "source": source,
            "target": self.target(target_stage),
            "summary": "Continue the workflow.",
        }
        if direction == "plan_to_task":
            contract["affected_ids"] = ["GOAL-001"]
        elif direction in {"execute_to_task", "task_to_plan", "execute_to_plan"}:
            contract.update(
                {
                    "confirmed_approach": "Update the source artifact.",
                    "requested_changes": ["Clarify the expected behavior."],
                    "preserve": ["Keep existing identifiers."],
                    "affected_ids": ["TASK-001"],
                    "validation_requirements": ["Revalidate the artifact."],
                }
            )
        return contract

    def test_accepts_all_handoff_directions_with_instruction_fingerprint(self) -> None:
        for direction in (
            "plan_to_task",
            "task_to_execute",
            "execute_to_task",
            "task_to_plan",
            "execute_to_plan",
        ):
            with self.subTest(direction=direction):
                result = validate_handoff_contract(
                    self.handoff(direction),
                    project_root=self.project_root,
                )

                self.assertEqual(result["status"], "valid")

    def test_rejects_legacy_task_rules_fingerprint(self) -> None:
        contract = self.handoff("task_to_execute")
        source = contract["source"]
        assert isinstance(source, dict)
        source["task_rules_sha256"] = source.pop("task_instructions_sha256")

        with self.assertRaises(WorkError) as context:
            validate_handoff_contract(contract, project_root=self.project_root)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["task_instructions_sha256"],
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["task_rules_sha256"],
        )

    def test_rejects_legacy_target_hierarchy(self) -> None:
        contract = self.handoff("task_to_execute")
        target = contract["target"]
        assert isinstance(target, dict)
        target["selection_topology"] = ["general", "web"]

        with self.assertRaises(WorkError) as context:
            validate_handoff_contract(contract, project_root=self.project_root)

        self.assertEqual(
            context.exception.code,
            "invalid_object_fields",
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["selection_topology"],
        )

    def test_rejects_missing_skill_selection_fingerprint(self) -> None:
        contract = self.handoff("task_to_execute")
        source = contract["source"]
        assert isinstance(source, dict)
        source.pop("skill_selection_sha256")

        with self.assertRaises(WorkError) as context:
            validate_handoff_contract(contract, project_root=self.project_root)

        self.assertEqual(
            context.exception.code,
            "invalid_object_fields",
        )
        self.assertEqual(context.exception.details["missing"], ["skill_selection_sha256"])


if __name__ == "__main__":
    unittest.main()
