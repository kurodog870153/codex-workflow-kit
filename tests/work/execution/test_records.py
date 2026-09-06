from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.records import formal_record_kind, next_record_id
from worklib.foundation.errors import ExitCode, WorkError


class ExecutionRecordTests(unittest.TestCase):
    def test_formal_record_kind_matches_each_task_collection(self) -> None:
        task = {
            "commands": [{"id": "CMD-001"}],
            "operations": [{"id": "OP-001"}],
            "validations": [{"id": "VAL-001"}],
        }
        for record_id, expected in (
            ("CMD-001", "command"),
            ("OP-001", "operation"),
            ("VAL-001", "validation"),
        ):
            with self.subTest(record_id=record_id):
                self.assertEqual(formal_record_kind(task, record_id), expected)

    def test_missing_formal_record_preserves_error_contract(self) -> None:
        with self.assertRaises(WorkError) as context:
            formal_record_kind({"commands": [{"id": "CMD-002"}]}, "CMD-001")
        self.assertEqual(context.exception.exit_code, ExitCode.CONTRACT)
        self.assertEqual(context.exception.code, "record_begin_record_not_found")
        self.assertEqual(context.exception.details, {"record_id": "CMD-001"})

    def test_new_record_uses_base_identifier(self) -> None:
        self.assertEqual(next_record_id("VAL-001", {"records": []}), "VAL-001")

    def test_retry_uses_highest_carried_or_current_instance(self) -> None:
        attempt = {
            "carried_records": [{"record_id": "VAL-001#3"}],
            "records": [{"id": "VAL-001"}, {"id": "VAL-001#1"}, {"id": "VAL-002#9"}],
        }
        self.assertEqual(next_record_id("VAL-001", attempt), "VAL-001#4")
        self.assertEqual(next_record_id("VAL-003", attempt), "VAL-003")

    def test_invalid_base_identifier_preserves_error_contract(self) -> None:
        for record_id in ("VAL-001#1", "UNKNOWN-001", "", None):
            with self.subTest(record_id=record_id):
                with self.assertRaises(WorkError) as context:
                    next_record_id(record_id, {})
                self.assertEqual(context.exception.exit_code, ExitCode.CONTRACT)
                self.assertEqual(
                    context.exception.code, "record_begin_invalid_base_record_id"
                )
                self.assertEqual(context.exception.details, {"record_id": record_id})


if __name__ == "__main__":
    unittest.main()
