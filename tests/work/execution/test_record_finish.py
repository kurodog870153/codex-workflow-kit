from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.business_services.execution.record_finish import build_finished_attempt
from worklib.models.common.errors import WorkError
from worklib.services.record.result import finish_attempt_candidate, overall_operation_result


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
                self.assertEqual(overall_operation_result(records), expected)

    def test_skipped_operation_is_not_reported_as_success(self) -> None:
        self.assertIsNone(overall_operation_result([{
            "id": "OP-001", "kind": "operation", "status": "skipped",
            "reason": "Not applicable.", "deviation_id": "DEVIATION-001",
        }]))

    def test_derives_record_identity_and_rejects_machine_fields(self) -> None:
        attempt = {"records": []}
        candidate = finish_attempt_candidate(
            attempt, {"record": {"outcome": "passed", "evidence": "Checked."}},
            expected_record_id="VAL-001", expected_kind="validation",
        )
        self.assertEqual(candidate["records"][0]["id"], "VAL-001")
        self.assertEqual(candidate["records"][0]["kind"], "validation")
        cases = (
            (
                {"record": {"id": "VAL-002"}},
                "record_finish_machine_fields",
            ),
            (
                {"record": {"kind": "operation"}},
                "record_finish_machine_fields",
            ),
            (
                {
                    "record": {
                        "correction": {},
                    }
                },
                "record_finish_machine_fields",
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
