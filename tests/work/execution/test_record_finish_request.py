from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.record_finish_request import parse_record_finish_request
from worklib.foundation.errors import WorkError


class RecordFinishRequestTests(unittest.TestCase):
    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_record_finish_request(
            json.dumps(request).encode("utf-8"),
            source="stdin",
        )

    def test_parses_record_and_optional_modified_files(self) -> None:
        request = {
            "schema": "work-record-finish-request/v1",
            "record": {"id": "VAL-001", "kind": "validation"},
            "modified_files": ["src/app.py"],
        }

        self.assertEqual(self.parse(request), request)

    def test_rejects_invalid_request_values(self) -> None:
        base = {
            "schema": "work-record-finish-request/v1",
            "record": {"id": "VAL-001", "kind": "validation"},
        }
        cases = (
            ({**base, "schema": "invalid"}, "record_finish_invalid_schema"),
            ({**base, "record": []}, "record_finish_invalid_record"),
            ({**base, "modified_files": []}, "record_finish_invalid_modified_files"),
            ({**base, "modified_files": [""]}, "record_finish_invalid_modified_file"),
        )
        for request, expected_code in cases:
            with self.subTest(expected_code=expected_code):
                with self.assertRaises(WorkError) as context:
                    self.parse(request)

                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
