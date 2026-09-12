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
from worklib.foundation.errors import ExitCode


class PlanCliTests(FileInputTestCase):
    def test_validate_input_file_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "plan",
                "validate",
                "--user-config-root",
                "/config",
                "--skill-root",
                "repo:.agents/skills=/skills",
                "--input-file", "request.json",
                "--plan-path",
                "outputs/work/plans/example.json",
            ]
        )

        self.assertEqual(arguments.command, "plan")
        self.assertEqual(arguments.plan_command, "validate")
        self.assertEqual(arguments.project_root, "/project")
        self.assertEqual(arguments.user_config_root, "/config")
        self.assertEqual(
            arguments.skill_root,
            ["repo:.agents/skills=/skills"],
        )
        self.assertTrue(arguments.input_file)
        self.assertIsNone(arguments.path)
        self.assertEqual(
            arguments.plan_path,
            "outputs/work/plans/example.json",
        )

    def test_validate_input_file_requires_plan_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                self.input_arguments([
                    "--project-root",
                    project_directory,
                    "plan",
                    "validate",
                    "--user-config-root",
                    project_directory,
                    "--input-file", "request.json",
                ], "{}"),
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stderr.getvalue(), "")
        error = json.loads(stdout.getvalue())
        self.assertEqual(error["schema"], "work-cli-result/v1")
        self.assertEqual(error["reason_code"], "plan_path_required")

    def test_validate_file_rejects_plan_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "plan",
                    "validate",
                    "--user-config-root",
                    project_directory,
                    "--path",
                    "outputs/work/plans/example.json",
                    "--plan-path",
                    "outputs/work/plans/other.json",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stderr.getvalue(), "")
        error = json.loads(stdout.getvalue())
        self.assertEqual(error["schema"], "work-cli-result/v1")
        self.assertEqual(error["reason_code"], "unexpected_plan_path")


if __name__ == "__main__":
    unittest.main()
