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
from worklib.instructions.selection import build_instruction_selection
from worklib.contracts.plan import render_plan_contract, validate_plan_file
from worklib.skills.selection import selection_sha256
from worklib.contracts.task import render_task_contract, validate_task_contract
from worklib.instructions.task_selection import (
    build_task_document_instruction_selection,
)
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.hierarchy.selection import build_hierarchy_selection


class TaskInstructionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.project_root = Path(self.temporary_directory.name).resolve()
        (self.project_root / "src.txt").write_text("source\n", encoding="utf-8")
        self.artifacts = {
            "plan": "outputs/work/plans/example.md",
            "task": "outputs/work/tasks/example.md",
            "execution": "outputs/work/executions/example",
        }
        hierarchy_selection = build_hierarchy_selection(
            {"decision": "general_only", "selections": []},
            skill_root=SKILL_ROOT,
        )
        plan = {
            "schema": "work-plan/v1",
            "requirement_id": "example",
            "status": "confirmed",
            "title": "Example Plan",
            "summary": "Plan summary.",
            "artifacts": self.artifacts,
            "hierarchy_selection": hierarchy_selection,
            "work_instruction_selection": build_work_instruction_selection(
                skill_root=SKILL_ROOT,
                mode="plan",
                selected_paths=hierarchy_selection["selected_paths"],
            ),
            "skill_selection": {
                "schema": "work-skill-selection/v1", "decision": "base_only",
                "skills": [], "selection_sha256": selection_sha256("base_only", []),
            },
            "goals": [{"id": "GOAL-001", "statement": "Complete the goal."}],
            "scope": [
                {
                    "id": "SCOPE-001",
                    "kind": "in_scope",
                    "statement": "Handle the scope.",
                    "goal_ids": ["GOAL-001"],
                }
            ],
            "deliverables": [
                {
                    "id": "DELIVERABLE-001",
                    "statement": "Produce the result.",
                    "goal_ids": ["GOAL-001"],
                    "acceptance_ids": ["ACCEPTANCE-001"],
                }
            ],
            "acceptance_criteria": [
                {
                    "id": "ACCEPTANCE-001",
                    "statement": "The result is observable.",
                    "deliverable_ids": ["DELIVERABLE-001"],
                }
            ],
        }
        plan_path = self.project_root / self.artifacts["plan"]
        plan_path.parent.mkdir(parents=True)
        plan_path.write_bytes(render_plan_contract(plan))
        plan_validation = validate_plan_file(
            self.project_root,
            str(self.project_root),
            self.artifacts["plan"],
        )

        task_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="task",
            selected_paths=[],
            reference_names=["task.general.task-records"],
        )
        self.contract: dict[str, object] = {
            "schema": "work-task/v1",
            "requirement_id": "example",
            "spec_id": "TASK-SPEC-001",
            "status": "confirmed",
            "title": "Example TASK",
            "summary": "Task summary.",
            "artifacts": self.artifacts,
            "source_plan": {
                "canonical_sha256": plan_validation["plan_sha256"],
                "hierarchy_selection_sha256": plan_validation[
                    "hierarchy_selection_sha256"
                ],
            },
            "instruction_selection": build_task_document_instruction_selection(
                [task_selection],
                skill_root=SKILL_ROOT,
            ),
            "execution_defaults": {
                "working_directory": ".",
                "os": "windows",
                "shell": "powershell",
            },
            "tasks": [
                {
                    "id": "TASK-001",
                    "title": "Modify source",
                    "skill_id": None,
                    "instruction_selection": task_selection,
                    "traceability": {
                        "goal_ids": ["GOAL-001"],
                        "deliverable_ids": ["DELIVERABLE-001"],
                        "acceptance_ids": ["ACCEPTANCE-001"],
                    },
                    "goal": "Modify the source and validate it.",
                    "files": [
                        {
                            "id": "FILE-001",
                            "action": "modify",
                            "path": "src.txt",
                        }
                    ],
                    "steps": [
                        {
                            "id": "STEP-001",
                            "action": "Modify the source.",
                            "references": ["FILE-001"],
                        },
                        {
                            "id": "STEP-002",
                            "action": "Run validation.",
                            "references": ["CMD-001", "VAL-001"],
                        },
                    ],
                    "commands": [
                        {
                            "id": "CMD-001",
                            "mode": "argv",
                            "argv": ["python", "--version"],
                        }
                    ],
                    "validations": [
                        {
                            "id": "VAL-001",
                            "kind": "automated",
                            "command_ids": ["CMD-001"],
                            "pass_condition": "Exit code is zero.",
                            "acceptance_ids": ["ACCEPTANCE-001"],
                        }
                    ],
                }
            ],
            "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
        }

    def validate(self, contract: dict[str, object] | None = None) -> dict[str, object]:
        value = contract or self.contract
        return validate_task_contract(
            render_task_contract(value),
            source="test",
            actual_task_path=self.artifacts["task"],
            project_root=self.project_root,
            user_config_root=str(self.project_root),
        )

    def test_valid_task_returns_instruction_fingerprints(self) -> None:
        result = self.validate()

        self.assertEqual(
            result["instructions_sha256"],
            self.contract["instruction_selection"]["instructions_sha256"],  # type: ignore[index]
        )
        task = self.contract["tasks"][0]  # type: ignore[index]
        self.assertEqual(
            result["task_instructions_sha256"],
            {"TASK-001": task["instruction_selection"]["instructions_sha256"]},
        )
        self.assertEqual(result["task_skill_ids"], {"TASK-001": None})
        self.assertEqual(
            result["hierarchy_selection_sha256"],
            self.contract["source_plan"]["hierarchy_selection_sha256"],  # type: ignore[index]
        )
        self.assertNotIn("rules_sha256", result)
        self.assertNotIn("task_rules_sha256", result)

    def test_render_orders_instruction_selections_canonically(self) -> None:
        raw = render_task_contract(dict(reversed(self.contract.items())))
        payload = json.loads(
            raw.decode("utf-8").split("```json\n", 1)[1].rsplit("\n```", 1)[0]
        )

        self.assertEqual(list(payload)[8], "instruction_selection")
        self.assertEqual(
            list(payload["instruction_selection"]),
            ["sources", "references", "instructions_sha256"],
        )
        self.assertEqual(
            list(payload["tasks"][0]["instruction_selection"]),
            [
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256",
            ],
        )
        self.assertEqual(list(payload["tasks"][0])[2], "skill_id")

    def test_rejects_skill_not_selected_in_plan(self) -> None:
        contract = copy.deepcopy(self.contract)
        task = contract["tasks"][0]  # type: ignore[index]
        task["skill_id"] = "missing"

        with self.assertRaises(WorkError) as context:
            self.validate(contract)

        self.assertEqual(context.exception.code, "task_skill_not_selected_in_plan")

    def test_rejects_legacy_document_rule_selection(self) -> None:
        contract = copy.deepcopy(self.contract)
        contract["rule_selection"] = contract.pop("instruction_selection")

        with self.assertRaises(WorkError) as context:
            self.validate(contract)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["instruction_selection"])
        self.assertEqual(context.exception.details["unknown"], ["rule_selection"])

    def test_rejects_legacy_task_rule_selection(self) -> None:
        contract = copy.deepcopy(self.contract)
        task = contract["tasks"][0]  # type: ignore[index]
        task["rule_selection"] = task.pop("instruction_selection")

        with self.assertRaises(WorkError) as context:
            self.validate(contract)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["instruction_selection"])
        self.assertEqual(context.exception.details["unknown"], ["rule_selection"])

    def test_rejects_stale_task_instruction_fingerprint(self) -> None:
        contract = copy.deepcopy(self.contract)
        task = contract["tasks"][0]  # type: ignore[index]
        task["instruction_selection"]["instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            self.validate(contract)

        self.assertEqual(context.exception.code, "instructions_fingerprint_mismatch")


if __name__ == "__main__":
    unittest.main()
