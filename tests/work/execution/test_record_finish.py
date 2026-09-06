from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.record_finish import (
    _overall_result,
    build_finished_attempt,
)
from worklib.foundation.errors import WorkError


class RecordFinishTests(unittest.TestCase):
    def test_aggregates_operation_outcomes(self) -> None:
        cases = (
            ([], None),
            (
                [{"id": "OP-001", "kind": "operation", "outcome": "success"}],
                {"status": "complete_success", "effective": ["OP-001"]},
            ),
            (
                [
                    {"id": "OP-001", "kind": "operation", "outcome": "success"},
                    {"id": "OP-002", "kind": "operation", "outcome": "failure"},
                ],
                {
                    "status": "partial_success",
                    "effective": ["OP-001"],
                    "not_effective": ["OP-002"],
                },
            ),
            (
                [{"id": "OP-001", "kind": "operation", "outcome": "unknown"}],
                {"status": "uncertain_result", "unknown": ["OP-001"]},
            ),
        )
        for records, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(_overall_result(records), expected)

    def test_rejects_record_that_does_not_match_reservation(self) -> None:
        attempt = {"records": []}
        cases = (
            (
                {"record": {"id": "VAL-002", "kind": "validation"}},
                "record_finish_record_id_mismatch",
            ),
            (
                {"record": {"id": "VAL-001", "kind": "operation"}},
                "record_finish_record_kind_mismatch",
            ),
            (
                {
                    "record": {
                        "id": "VAL-001",
                        "kind": "validation",
                        "correction": {},
                    }
                },
                "record_finish_untrusted_command_correction",
            ),
        )
        for request, expected_code in cases:
            with self.subTest(expected_code=expected_code):
                with self.assertRaises(WorkError) as context:
                    build_finished_attempt(
                        attempt,
                        request,
                        project_root=Path.cwd(),
                        expected_record_id="VAL-001",
                        expected_kind="validation",
                    )

                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
