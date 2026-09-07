from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.correction_request import parse_correction_create_request
from worklib.foundation.errors import WorkError


class CorrectionRequestTests(unittest.TestCase):
    def request(self) -> dict[str, object]:
        return {
            "schema": "work-correction-create-request/v1",
            "target_attempt_id": "ATTEMPT-001",
            "field": "records[0].outcome",
            "correct_value": "passed",
            "reason": "Correct the recorded outcome.",
            "invalidates_completion": True,
        }

    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_correction_create_request(
            json.dumps(request).encode("utf-8"),
            source="stdin",
        )

    def test_accepts_correction_create_request(self) -> None:
        request = self.request()

        self.assertEqual(self.parse(request), request)

    def test_rejects_invalid_schema_and_attempt_id(self) -> None:
        cases = (
            ("schema", "invalid", "correction_create_invalid_schema"),
            (
                "target_attempt_id",
                "ATTEMPT-1",
                "correction_create_invalid_attempt_id",
            ),
        )
        for field, value, expected_code in cases:
            with self.subTest(field=field):
                request = self.request()
                request[field] = value

                with self.assertRaises(WorkError) as context:
                    self.parse(request)

                self.assertEqual(context.exception.code, expected_code)

    def test_rejects_blank_text_fields(self) -> None:
        for field in ("field", "correct_value", "reason"):
            with self.subTest(field=field):
                request = self.request()
                request[field] = " "

                with self.assertRaises(WorkError) as context:
                    self.parse(request)

                self.assertEqual(context.exception.code, "correction_create_empty_text")
                self.assertEqual(context.exception.details["field"], field)

    def test_rejects_nonboolean_invalidation_flag(self) -> None:
        request = self.request()
        request["invalidates_completion"] = 1

        with self.assertRaises(WorkError) as context:
            self.parse(request)

        self.assertEqual(
            context.exception.code,
            "correction_create_invalid_invalidation_flag",
        )

    def test_rejects_missing_and_unknown_fields(self) -> None:
        request = self.request()
        request.pop("reason")
        request["extra"] = True

        with self.assertRaises(WorkError) as context:
            self.parse(request)

        self.assertEqual(context.exception.code, "correction_create_invalid_fields")
        self.assertEqual(context.exception.details["missing"], ["reason"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])


if __name__ == "__main__":
    unittest.main()
