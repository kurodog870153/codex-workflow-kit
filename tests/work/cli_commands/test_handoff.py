from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import build_parser, main
from worklib.foundation.errors import ExitCode, WorkError


class HandoffCliTests(unittest.TestCase):
    def test_validate_stdin_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "handoff",
                "validate",
                "--stdin",
            ]
        )

        self.assertEqual(arguments.command, "handoff")
        self.assertEqual(arguments.handoff_command, "validate")
        self.assertTrue(arguments.stdin)

    def test_render_stdin_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "handoff",
                "render",
                "--stdin",
            ]
        )

        self.assertEqual(arguments.handoff_command, "render")
        self.assertTrue(arguments.stdin)

    def test_validate_requires_stdin(self) -> None:
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
                [
                    "--project-root",
                    project_directory,
                    "handoff",
                    "validate",
                    "--stdin",
                ],
                stdin=io.StringIO("{"),
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.INPUT_FORMAT)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "invalid_json_contract")


if __name__ == "__main__":
    unittest.main()
