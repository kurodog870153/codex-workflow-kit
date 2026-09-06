from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.attempt_close_request import parse_attempt_close_request
from worklib.foundation.errors import WorkError


class AttemptCloseRequestTests(unittest.TestCase):
    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_attempt_close_request(
            json.dumps(request).encode("utf-8"),
            source="stdin",
        )

    def test_accepts_completed_request_without_final_details(self) -> None:
        request = {
            "schema": "work-attempt-close-request/v1",
            "status": "completed",
        }

        self.assertEqual(self.parse(request), request)

    def test_accepts_stopped_and_blocked_requests_with_final_details(self) -> None:
        for status in ("stopped", "blocked"):
            with self.subTest(status=status):
                request = {
                    "schema": "work-attempt-close-request/v1",
                    "status": status,
                    "final_type": "other",
                    "reason": "Recorded reason.",
                }

                self.assertEqual(self.parse(request), request)

    def test_rejects_invalid_status(self) -> None:
        with self.assertRaises(WorkError) as context:
            self.parse(
                {
                    "schema": "work-attempt-close-request/v1",
                    "status": "in_progress",
                }
            )

        self.assertEqual(context.exception.code, "attempt_close_invalid_status")

    def test_completed_request_rejects_final_details(self) -> None:
        with self.assertRaises(WorkError) as context:
            self.parse(
                {
                    "schema": "work-attempt-close-request/v1",
                    "status": "completed",
                    "final_type": "other",
                    "reason": "Unexpected.",
                }
            )

        self.assertEqual(
            context.exception.code,
            "attempt_close_unexpected_final_details",
        )
        self.assertEqual(context.exception.details["fields"], ["final_type", "reason"])

    def test_noncompleted_request_requires_final_details(self) -> None:
        with self.assertRaises(WorkError) as context:
            self.parse(
                {
                    "schema": "work-attempt-close-request/v1",
                    "status": "stopped",
                }
            )

        self.assertEqual(context.exception.code, "attempt_close_missing_final_details")
        self.assertEqual(context.exception.details["missing"], ["final_type", "reason"])

    def test_noncompleted_request_rejects_blank_reason(self) -> None:
        with self.assertRaises(WorkError) as context:
            self.parse(
                {
                    "schema": "work-attempt-close-request/v1",
                    "status": "blocked",
                    "final_type": "other",
                    "reason": " ",
                }
            )

        self.assertEqual(context.exception.code, "attempt_close_empty_final_detail")

    def test_rejects_missing_and_unknown_fields(self) -> None:
        with self.assertRaises(WorkError) as context:
            self.parse({"schema": "work-attempt-close-request/v1", "extra": True})

        self.assertEqual(context.exception.code, "attempt_close_invalid_fields")
        self.assertEqual(context.exception.details["missing"], ["status"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])


if __name__ == "__main__":
    unittest.main()
