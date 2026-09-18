from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from pydantic import ValidationError

from worklib.models.execution.command import (
    CommandCorrectionRequestContract, CommandPreviewContract,
)
from worklib.models.execution.command import (
    CommandCorrectionRequestContract as LegacyCommandCorrectionRequestContract,
)
from worklib.models.common.errors import WorkError
from worklib.services.command.validation import parse_command_correction_request


class CommandCorrectionRequestTests(unittest.TestCase):
    def test_legacy_export_preserves_class_identity(self) -> None:
        self.assertIs(LegacyCommandCorrectionRequestContract, CommandCorrectionRequestContract)

    def request(self) -> dict[str, object]:
        return {
            "schema": "work-command-correction-request/v1",
            "record_id": "CMD-001#2",
            "original_command": {"mode": "argv", "argv": ["tool", "old"]},
            "actual_command": {"mode": "argv", "argv": ["tool", "new"]},
            "reason": "Use the authorized argument.",
        }

    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_command_correction_request(
            json.dumps(request).encode("utf-8"), source="stdin"
        ).to_execution_dict()

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


class CommandPreviewContractTests(unittest.TestCase):
    def test_accepts_direct_and_windows_batch_invocations(self) -> None:
        direct = dict(CommandPreviewContract.contract_example)
        batch = dict(direct)
        batch["invocation"] = {
            "kind": "windows_batch", "launcher": "C:/Windows/System32/cmd.exe",
            "launcher_sha256": "1" * 64, "script": "C:/tools/test.cmd",
            "script_sha256": "2" * 64, "arguments": ["two words"],
            "command_line": '"C:/tools/test.cmd" "two words"',
            "launcher_arguments": ["/d", "/s", "/v:off", "/c", '"C:/tools/test.cmd" "two words"'],
        }
        for preview in (direct, batch):
            with self.subTest(kind=preview["invocation"]["kind"]):
                parsed = CommandPreviewContract.model_validate(preview).to_canonical_dict()
                self.assertEqual(parsed["invocation"]["kind"], preview["invocation"]["kind"])

    def test_rejects_legacy_and_mixed_invocation_fields(self) -> None:
        legacy = dict(CommandPreviewContract.contract_example)
        legacy.pop("invocation")
        legacy.update(argv=["tool"], selected_executable="tool", executable_sha256="0" * 64)
        mixed = dict(CommandPreviewContract.contract_example)
        mixed["invocation"] = {**mixed["invocation"], "script": "tool.cmd"}
        for preview in (legacy, mixed):
            with self.subTest(preview=preview):
                with self.assertRaises(ValidationError):
                    CommandPreviewContract.model_validate(preview)


if __name__ == "__main__":
    unittest.main()
