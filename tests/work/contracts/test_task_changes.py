from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.task_changes import validate_task_changes
from worklib.foundation.errors import WorkError


class TaskChangeTests(unittest.TestCase):
    def change(self) -> dict[str, object]:
        return {
            "id": "TASK-CHANGE-001",
            "spec_id": "TASK-SPEC-001",
            "date": "2026-09-07",
            "reason": "Update task details.",
            "affected_ids": ["TASK-001", "TASK-001/FILE-001"],
            "plan_change_ids": ["PLAN-CHANGE-001"],
            "edits": [
                {"operation": "add", "path": "/summary", "after": "New"},
                {
                    "operation": "replace",
                    "path": "/goal",
                    "before": "Old",
                    "after": "New",
                },
                {"operation": "remove", "path": "/risks/0", "before": {}},
            ],
        }

    def validate(self, changes: object) -> None:
        validate_task_changes(changes, "TASK-SPEC-001", {"TASK-001"})

    def test_accepts_ordered_changes_and_all_edit_operations(self) -> None:
        self.validate([self.change()])

    def test_rejects_unsorted_change_ids(self) -> None:
        first = self.change()
        first["id"] = "TASK-CHANGE-002"
        second = self.change()

        with self.assertRaises(WorkError) as context:
            self.validate([first, second])

        self.assertEqual(context.exception.code, "invalid_or_unsorted_id")

    def test_rejects_spec_mismatch_and_invalid_date(self) -> None:
        cases = (
            ("spec_id", "TASK-SPEC-002", "change_spec_mismatch"),
            ("date", "2026-02-30", "invalid_change_date"),
        )
        for field, value, expected_code in cases:
            with self.subTest(field=field):
                change = self.change()
                change[field] = value

                with self.assertRaises(WorkError) as context:
                    self.validate([change])

                self.assertEqual(context.exception.code, expected_code)

    def test_rejects_unknown_affected_and_invalid_plan_change_ids(self) -> None:
        cases = (
            ("affected_ids", ["TASK-999"]),
            ("plan_change_ids", ["CHANGE-001"]),
        )
        for field, value in cases:
            with self.subTest(field=field):
                change = self.change()
                change[field] = value

                with self.assertRaises(WorkError) as context:
                    self.validate([change])

                self.assertEqual(context.exception.code, "invalid_reference")

    def test_rejects_invalid_edit_fields_and_json_pointer(self) -> None:
        invalid_fields = self.change()
        invalid_fields["edits"] = [
            {"operation": "add", "path": "/summary", "before": "Old"}
        ]
        invalid_pointer = self.change()
        invalid_pointer["edits"] = [
            {"operation": "add", "path": "summary", "after": "New"}
        ]
        cases = (
            (invalid_fields, "invalid_change_edit"),
            (invalid_pointer, "invalid_json_pointer"),
        )
        for change, expected_code in cases:
            with self.subTest(expected_code=expected_code):
                with self.assertRaises(WorkError) as context:
                    self.validate([copy.deepcopy(change)])

                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
