from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.completion import validate_completed_coverage
from worklib.foundation.errors import WorkError


class CompletionTests(unittest.TestCase):
    def test_accepts_when_all_formal_validations_pass(self) -> None:
        validate_completed_coverage(
            task={"validations": [{"id": "VAL-001"}, {"id": "VAL-002"}]},
            attempt={
                "records": [
                    {"id": "VAL-001", "kind": "validation", "outcome": "passed"},
                    {"id": "VAL-002", "kind": "validation", "outcome": "passed"},
                ]
            },
        )

    def test_reports_missing_and_failed_validations_in_task_order(self) -> None:
        with self.assertRaises(WorkError) as context:
            validate_completed_coverage(
                task={
                    "validations": [
                        {"id": "VAL-001"},
                        {"id": "VAL-002"},
                        {"id": "VAL-003"},
                    ]
                },
                attempt={
                    "records": [
                        {
                            "id": "VAL-001",
                            "kind": "validation",
                            "outcome": "failed",
                        },
                        {
                            "id": "VAL-003",
                            "kind": "validation",
                            "outcome": "passed",
                        },
                    ]
                },
            )

        self.assertEqual(context.exception.code, "attempt_close_incomplete_validations")
        self.assertEqual(context.exception.details["missing"], ["VAL-002"])
        self.assertEqual(context.exception.details["failed"], ["VAL-001"])

    def test_carried_validation_counts_as_passed(self) -> None:
        validate_completed_coverage(
            task={"validations": [{"id": "VAL-001"}]},
            attempt={
                "carried_records": [{"record_id": "VAL-001"}],
                "records": [],
            },
        )

    def test_current_retry_overrides_carried_validation(self) -> None:
        with self.assertRaises(WorkError) as context:
            validate_completed_coverage(
                task={"validations": [{"id": "VAL-001"}]},
                attempt={
                    "carried_records": [{"record_id": "VAL-001"}],
                    "records": [
                        {
                            "id": "VAL-001#2",
                            "kind": "validation",
                            "outcome": "failed",
                        }
                    ],
                },
            )

        self.assertEqual(context.exception.details["missing"], [])
        self.assertEqual(context.exception.details["failed"], ["VAL-001"])


if __name__ == "__main__":
    unittest.main()
