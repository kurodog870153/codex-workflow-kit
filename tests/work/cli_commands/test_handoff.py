from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.cli import build_parser, main
from worklib.foundation.errors import ExitCode, WorkError


class HandoffCliTests(FileInputTestCase):
    def test_validate_input_file_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "handoff",
                "validate",
                "--input-file", "request.json",
            ]
        )

        self.assertEqual(arguments.command, "handoff")
        self.assertEqual(arguments.handoff_command, "validate")
        self.assertTrue(arguments.input_file)

    def test_render_input_file_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "handoff",
                "render",
                "--input-file", "request.json",
            ]
        )

        self.assertEqual(arguments.handoff_command, "render")
        self.assertTrue(arguments.input_file)

    def test_validate_requires_input_file(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(
                [
                    "--project-root",
                    "/project",
                    "handoff",
                    "validate",
                ]
            )

        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_validate_invalid_json_uses_input_format_error(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                self.input_arguments([
                    "--project-root",
                    project_directory,
                    "handoff",
                    "validate",
                    "--input-file", "request.json",
                ], "{"),
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.INPUT_FORMAT)
        self.assertEqual(stderr.getvalue(), "")
        error = json.loads(stdout.getvalue())
        self.assertEqual(error["schema"], "work-cli-result/v1")
        self.assertEqual(error["reason_code"], "invalid_json_contract")


if __name__ == "__main__":
    unittest.main()
