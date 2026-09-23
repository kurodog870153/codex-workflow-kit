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
            "actual_command": {"mode": "argv", "argv": ["tool", "new"]},
            "reason": "Use the authorized argument.",
        }

    def parse(self, request: dict[str, object]) -> dict[str, object]:
        return parse_command_correction_request(
            json.dumps(request).encode("utf-8"), source="stdin"
        ).to_canonical_dict()

    def test_parses_and_canonicalizes_request(self) -> None:
        parsed = self.parse(self.request())

        self.assertEqual(parsed["schema"], "work-command-correction-request/v1")
        self.assertEqual(parsed["actual_command"], {"mode": "argv", "argv": ["tool", "new"]})
        self.assertEqual(parsed["reason"], "Use the authorized argument.")

    def test_parses_shell_command(self) -> None:
        request = self.request()
        request["actual_command"] = {"mode": "shell", "script": "echo hello"}

        self.assertEqual(
            self.parse(request)["actual_command"],
            {"mode": "shell", "script": "echo hello"},
        )

    def test_rejects_invalid_command_shapes_and_values(self) -> None:
        commands = (
            {"mode": "argv", "script": "tool"},
            {"mode": "shell", "argv": ["tool"]},
            {"mode": "argv", "argv": ["tool"], "script": "tool"},
            {"mode": "argv", "argv": ["tool"], "id": "CMD-001"},
            {"mode": "argv", "argv": []},
            {"mode": "argv", "argv": [" "]},
            {"mode": "shell", "script": "  "},
            {"mode": "invalid", "argv": ["tool"]},
        )
        for command in commands:
            with self.subTest(command=command):
                request = self.request()
                request["actual_command"] = command
                with self.assertRaises(WorkError) as context:
                    self.parse(request)
                self.assertEqual(context.exception.code, "command_correction_invalid_fields")

    def test_schema_discriminates_command_modes(self) -> None:
        command_schema = CommandCorrectionRequestContract.model_json_schema()["properties"]["actual_command"]
        self.assertEqual(command_schema["discriminator"]["propertyName"], "mode")
        self.assertEqual(len(command_schema["oneOf"]), 2)
        definitions = CommandCorrectionRequestContract.model_json_schema()["$defs"]
        self.assertEqual(definitions["SemanticArgvCommandModel"]["required"], ["mode", "argv"])
        self.assertEqual(definitions["SemanticShellCommandModel"]["required"], ["mode", "script"])
        self.assertFalse(definitions["SemanticArgvCommandModel"]["additionalProperties"])
        self.assertFalse(definitions["SemanticShellCommandModel"]["additionalProperties"])

    def test_rejects_missing_and_unknown_fields(self) -> None:
        request = self.request()
        request.pop("reason")
        request["extra"] = True

        with self.assertRaises(WorkError) as context:
            self.parse(request)

        self.assertEqual(context.exception.code, "command_correction_invalid_fields")
        self.assertEqual(context.exception.details["missing"], ["reason"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])

    def test_rejects_invalid_schema_and_formal_identity(self) -> None:
        cases = (
            ("schema", "invalid", "command_correction_invalid_schema"),
            ("record_id", "CMD-001", "command_correction_invalid_fields"),
            ("original_command", {"mode": "argv", "argv": ["tool", "old"]}, "command_correction_invalid_fields"),
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
