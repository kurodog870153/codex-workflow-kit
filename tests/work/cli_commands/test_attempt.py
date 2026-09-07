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


class AttemptCliTests(unittest.TestCase):
    def test_validate_path_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "attempt",
                "validate",
                "--path",
                "outputs/work/executions/example/ATTEMPT-001.md",
            ]
        )

        self.assertEqual(arguments.command, "attempt")
        self.assertEqual(arguments.attempt_command, "validate")
        self.assertEqual(
            arguments.path,
            "outputs/work/executions/example/ATTEMPT-001.md",
        )
        self.assertFalse(arguments.stdin)

    def test_validate_rejects_path_and_stdin(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(
                [
                    "--project-root",
                    "/project",
                    "attempt",
                    "validate",
                    "--path",
                    "attempt.md",
                    "--stdin",
                ]
            )

        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_render_requires_stdin(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(
                [
                    "--project-root",
                    "/project",
                    "attempt",
                    "render",
                ]
            )

        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_render_invalid_json_uses_input_format_error(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "attempt",
                    "render",
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
