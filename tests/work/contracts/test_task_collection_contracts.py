from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.business_services.task.index import (
    render_task_index_contract,
    validate_task_index_contract,
)
from worklib.business_services.task.item import (
    render_task_item_contract,
    validate_task_item_contract,
)
from worklib.models.common.errors import WorkError


def selection(*, document: bool = False) -> dict[str, object]:
    value: dict[str, object] = {
        "sources": [
            {
                "kind": "instruction",
                "logical_name": "task.general",
                "canonical_sha256": "a" * 64,
            }
        ],
        "references": ["task.general.task-records"],
        "instructions_sha256": "b" * 64,
    }
    if not document:
        value = {
            "selected_paths": [],
            "resolved_paths": ["skills/work/references/instructions/task/general/instructions.md"],
            **value,
        }
    return value


def item() -> dict[str, object]:
    return {
        "schema": "work-task-item/v1",
        "id": "TASK-001",
        "title": "Modify source",
        "skill_id": None,
        "instruction_selection": selection(),
        "traceability": {
            "goal_ids": ["GOAL-001"],
            "deliverable_ids": ["DELIVERABLE-001"],
            "acceptance_ids": ["ACCEPTANCE-001"],
        },
        "goal": "Modify the source and validate it.",
        "files": [{"id": "FILE-001", "action": "modify", "path": "src.txt"}],
        "steps": [
            {"id": "STEP-001", "action": "Modify source.", "references": ["FILE-001"]},
            {"id": "STEP-002", "action": "Validate.", "references": ["CMD-001", "VAL-001"]},
        ],
        "commands": [{"id": "CMD-001", "mode": "argv", "argv": ["python", "--version"]}],
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


class TaskCollectionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.project_root = Path(self.temporary_directory.name).resolve()
        self.item = item()
        self.item_raw = render_task_item_contract(self.item)
        item_sha = validate_task_item_contract(
            self.item_raw, source="test item", expected_task_id="TASK-001"
        )["task_item_sha256"]
        self.index: dict[str, object] = {
            "schema": "work-task-index/v1",
            "requirement_id": "example",
            "spec_id": "TASK-SPEC-001",
            "status": "confirmed",
            "title": "Example TASK",
            "summary": "Task summary.",
            "artifacts": {
                "plan": "outputs/work/plans/example.json",
                "task": "outputs/work/tasks/example/index.json",
                "execution": "outputs/work/executions/example",
            },
            "source_plan": {
                "canonical_sha256": "c" * 64,
                "hierarchy_selection_sha256": "d" * 64,
            },
            "instruction_selection": selection(document=True),
            "tasks": [
                {
                    "id": "TASK-001",
                    "path": "tasks/TASK-001.json",
                    "canonical_sha256": item_sha,
                }
            ],
            "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
        }

    def validate_index(self, value: dict[str, object] | None = None) -> dict[str, object]:
        contract = value or self.index
        return validate_task_index_contract(
            render_task_index_contract(contract),
            source="test index",
            actual_index_path="outputs/work/tasks/example/index.json",
            project_root=self.project_root,
        )

    def test_valid_index_and_item_return_fingerprints(self) -> None:
        item_validation = validate_task_item_contract(
            self.item_raw, source="test item", expected_task_id="TASK-001"
        )
        index_validation = self.validate_index()

        self.assertEqual(item_validation["task_id"], "TASK-001")
        self.assertEqual(index_validation["task_ids"], ["TASK-001"])
        self.assertEqual(
            index_validation["task_item_sha256"],
            {"TASK-001": item_validation["task_item_sha256"]},
        )

    def test_renderers_apply_canonical_field_order(self) -> None:
        rendered_item = render_task_item_contract(dict(reversed(self.item.items())))
        rendered_index = render_task_index_contract(dict(reversed(self.index.items())))

        self.assertTrue(rendered_item.startswith(b'{\n  "schema": "work-task-item/v1"'))
        self.assertIn(
            b'"id": "TASK-001",\n      "path": "tasks/TASK-001.json",\n      "canonical_sha256"',
            rendered_index,
        )

    def test_rejects_noncanonical_documents(self) -> None:
        for raw, validator in (
            (
                b" " + self.item_raw,
                lambda value: validate_task_item_contract(
                    value, source="test item", expected_task_id="TASK-001"
                ),
            ),
            (
                b" " + render_task_index_contract(self.index),
                lambda value: validate_task_index_contract(
                    value,
                    source="test index",
                    actual_index_path="outputs/work/tasks/example/index.json",
                    project_root=self.project_root,
                ),
            ),
        ):
            with self.subTest(validator=validator):
                with self.assertRaises(WorkError) as context:
                    validator(raw)
                self.assertEqual(context.exception.code, "noncanonical_json_contract")

    def test_rejects_item_identity_or_unknown_fields(self) -> None:
        wrong_id = copy.deepcopy(self.item)
        wrong_id["id"] = "TASK-002"
        with self.assertRaises(WorkError) as context:
            validate_task_item_contract(
                render_task_item_contract(wrong_id),
                source="test item",
                expected_task_id="TASK-001",
            )
        self.assertEqual(context.exception.code, "task_item_identity_mismatch")

        unknown = copy.deepcopy(self.item)
        unknown["unknown"] = True
        with self.assertRaises(WorkError) as context:
            validate_task_item_contract(
                render_task_item_contract(unknown),
                source="test item",
                expected_task_id="TASK-001",
            )
        self.assertEqual(context.exception.code, "invalid_object_fields")

    def test_rejects_unsorted_or_mismatched_index_references(self) -> None:
        second = copy.deepcopy(self.index["tasks"][0])  # type: ignore[index]
        second["id"] = "TASK-002"
        second["path"] = "tasks/TASK-002.json"
        unsorted = copy.deepcopy(self.index)
        unsorted["tasks"] = [second, unsorted["tasks"][0]]  # type: ignore[index]
        with self.assertRaises(WorkError) as context:
            self.validate_index(unsorted)
        self.assertEqual(context.exception.code, "invalid_or_unsorted_id")

        mismatched = copy.deepcopy(self.index)
        mismatched["tasks"][0]["path"] = "tasks/TASK-002.json"  # type: ignore[index]
        with self.assertRaises(WorkError) as context:
            self.validate_index(mismatched)
        self.assertEqual(context.exception.code, "task_item_path_mismatch")

    def test_change_history_increases_through_current_spec(self) -> None:
        revised = copy.deepcopy(self.index)
        revised["spec_id"] = "TASK-SPEC-003"
        revised["readiness"]["spec_id"] = "TASK-SPEC-003"  # type: ignore[index]
        revised["changes"] = [
            {
                "id": "TASK-CHANGE-001",
                "spec_id": "TASK-SPEC-002",
                "date": "2026-09-15",
                "reason": "First revision",
                "affected_ids": ["TASK-001"],
                "edits": [{
                    "artifact": "task_index",
                    "operation": "replace",
                    "path": "/summary",
                    "before": "Task summary.",
                    "after": "First summary.",
                }],
            },
            {
                "id": "TASK-CHANGE-002",
                "spec_id": "TASK-SPEC-003",
                "date": "2026-09-16",
                "reason": "Second revision",
                "affected_ids": ["TASK-001"],
                "edits": [{
                    "artifact": "task_index",
                    "operation": "replace",
                    "path": "/summary",
                    "before": "First summary.",
                    "after": "Second summary.",
                }],
            },
        ]
        self.validate_index(revised)

        revised["changes"][-1]["spec_id"] = "TASK-SPEC-002"  # type: ignore[index]
        with self.assertRaises(WorkError) as context:
            self.validate_index(revised)
        self.assertEqual(context.exception.code, "change_spec_mismatch")


if __name__ == "__main__":
    unittest.main()
