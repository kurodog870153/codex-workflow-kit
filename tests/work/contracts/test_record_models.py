from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.execution.record import RecordFinishRequestContract
from worklib.models.execution.record import RecordFinishRequestContract as LegacyRecordFinishRequestContract
from worklib.services.record.validation import parse_record_finish_request
from worklib.models.common.errors import WorkError


class RecordFinishRequestTests(unittest.TestCase):
    def test_legacy_export_preserves_class_identity(self) -> None:
        self.assertIs(LegacyRecordFinishRequestContract, RecordFinishRequestContract)

    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_record_finish_request(
            json.dumps(request).encode("utf-8"),
            source="stdin",
        ).to_canonical_dict()

    def test_parses_record_and_optional_modified_files(self) -> None:
        request = {
            "schema": "work-record-finish-request/v1",
            "record": {"outcome": "passed"},
            "modified_files": ["src/app.py"],
        }

        self.assertEqual(self.parse(request), request)

    def test_rejects_invalid_request_values(self) -> None:
        base = {
            "schema": "work-record-finish-request/v1",
            "record": {"outcome": "passed"},
        }
        cases = (
            ({**base, "schema": "invalid"}, "record_finish_invalid_schema"),
            ({**base, "record": []}, "record_finish_invalid_record"),
            ({**base, "modified_files": []}, "record_finish_invalid_modified_files"),
            ({**base, "modified_files": [""]}, "record_finish_invalid_modified_file"),
            ({**base, "record": {"id": "VAL-001"}}, "record_finish_machine_fields"),
            ({**base, "record": {"kind": "validation"}}, "record_finish_machine_fields"),
        )
        for request, expected_code in cases:
            with self.subTest(expected_code=expected_code):
                with self.assertRaises(WorkError) as context:
                    self.parse(request)

                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
