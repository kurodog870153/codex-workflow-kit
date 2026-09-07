from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.attempt_start_request import parse_attempt_start_request
from worklib.foundation.errors import WorkError


class AttemptStartRequestTests(unittest.TestCase):
    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_attempt_start_request(
            json.dumps(request).encode("utf-8"),
            source="stdin",
        )

    def base_request(self) -> dict[str, object]:
        return {
            "schema": "work-attempt-start-request/v1",
            "worktree_snapshot_sha256": "a" * 64,
        }

    def test_parses_request_without_continuation(self) -> None:
        request = self.base_request()

        self.assertEqual(self.parse(request), request)

    def test_canonicalizes_continuation_and_carried_records(self) -> None:
        request = self.base_request()
        request["continuation"] = {
            "source_attempt_id": "ATTEMPT-001",
            "carried_records": [
                {"record_id": "VAL-001", "evidence": "Passed previously."},
                {"record_id": "CMD-001#2", "evidence": "Authorized retry."},
            ],
        }

        self.assertEqual(self.parse(request), request)

    def test_rejects_invalid_schema_and_snapshot(self) -> None:
        cases = (
            ("schema", "invalid", "attempt_start_invalid_schema"),
            (
                "worktree_snapshot_sha256",
                "A" * 64,
                "attempt_start_invalid_worktree_snapshot",
            ),
        )
        for field, value, expected_code in cases:
            with self.subTest(field=field):
                request = self.base_request()
                request[field] = value

                with self.assertRaises(WorkError) as context:
                    self.parse(request)

                self.assertEqual(context.exception.code, expected_code)

    def test_rejects_duplicate_carried_record(self) -> None:
        request = self.base_request()
        request["continuation"] = {
            "source_attempt_id": "ATTEMPT-001",
            "carried_records": [
                {"record_id": "VAL-001", "evidence": "First."},
                {"record_id": "VAL-001", "evidence": "Duplicate."},
            ],
        }

        with self.assertRaises(WorkError) as context:
            self.parse(request)

        self.assertEqual(context.exception.code, "attempt_start_duplicate_carried_record")
        self.assertEqual(context.exception.details["record_id"], "VAL-001")

    def test_rejects_missing_and_unknown_fields(self) -> None:
        request = self.base_request()
        request.pop("worktree_snapshot_sha256")
        request["extra"] = True

        with self.assertRaises(WorkError) as context:
            self.parse(request)

        self.assertEqual(context.exception.code, "attempt_start_invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["worktree_snapshot_sha256"],
        )
        self.assertEqual(context.exception.details["unknown"], ["extra"])


if __name__ == "__main__":
    unittest.main()
