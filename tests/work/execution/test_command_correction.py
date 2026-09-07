from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.command_correction import parse_command_correction_request
from worklib.foundation.errors import WorkError


class CommandCorrectionTests(unittest.TestCase):
    def request(self) -> dict[str, object]:
        return {
            "schema": "work-command-correction-request/v1",
            "record_id": "CMD-001#2",
            "original_command": {"mode": "argv", "argv": ["tool", "old"]},
            "actual_command": {"mode": "argv", "argv": ["tool", "new"]},
            "reason": "Use the authorized argument.",
            "authorization_evidence": "User approved correction 1.",
        }

    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_command_correction_request(
            json.dumps(request).encode("utf-8"),
            source="stdin",
        )

    def test_parses_and_canonicalizes_request(self) -> None:
        parsed = self.parse(self.request())

        self.assertEqual(parsed["schema"], "work-command-correction-request/v1")
        self.assertEqual(parsed["record_id"], "CMD-001#2")
        self.assertEqual(
            parsed["correction"],
            {
                "original_command": {"mode": "argv", "argv": ["tool", "old"]},
                "actual_command": {"mode": "argv", "argv": ["tool", "new"]},
                "reason": "Use the authorized argument.",
                "authorization_evidence": "User approved correction 1.",
            },
        )

    def test_rejects_missing_and_unknown_fields(self) -> None:
        request = self.request()
        request.pop("reason")
        request["extra"] = True

        with self.assertRaises(WorkError) as context:
            self.parse(request)

        self.assertEqual(context.exception.code, "command_correction_invalid_fields")
        self.assertEqual(context.exception.details["missing"], ["reason"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])

    def test_rejects_invalid_schema_and_record_id(self) -> None:
        cases = (
            ("schema", "invalid", "command_correction_invalid_schema"),
            ("record_id", "OP-001", "command_correction_invalid_record_id"),
        )
        for field, value, expected_code in cases:
            with self.subTest(field=field):
                request = self.request()
                request[field] = value

                with self.assertRaises(WorkError) as context:
                    self.parse(request)

                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
