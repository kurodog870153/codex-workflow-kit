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


class AttemptCliTests(FileInputTestCase):
    def test_validate_path_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "attempt",
                "validate",
                "--path",
                "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
            ]
        )

        self.assertEqual(arguments.command, "attempt")
        self.assertEqual(arguments.attempt_command, "validate")
        self.assertEqual(
            arguments.path,
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
        )
        self.assertFalse(arguments.input_file)

    def test_validate_rejects_path_and_input_file(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(
                [
                    "--project-root",
                    "/project",
                    "attempt",
                    "validate",
                    "--path",
                    "attempt.json",
                    "--input-file", "request.json",
                ]
            )

        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_render_requires_input_file(self) -> None:
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
                self.input_arguments([
                    "--project-root",
                    project_directory,
                    "attempt",
                    "render",
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
