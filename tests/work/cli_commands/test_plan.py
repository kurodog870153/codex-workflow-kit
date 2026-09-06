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
from worklib.foundation.errors import ExitCode


class PlanCliTests(unittest.TestCase):
    def test_validate_stdin_arguments_parse(self) -> None:
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
                "--stdin",
                "--plan-path",
                "outputs/work/plans/example.md",
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
        self.assertTrue(arguments.stdin)
        self.assertIsNone(arguments.path)
        self.assertEqual(
            arguments.plan_path,
            "outputs/work/plans/example.md",
        )

    def test_validate_stdin_requires_plan_path(self) -> None:
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
                    "--stdin",
                ],
                stdin=io.StringIO("{}"),
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "plan_path_required")

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
                    "outputs/work/plans/example.md",
                    "--plan-path",
                    "outputs/work/plans/other.md",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "unexpected_plan_path")


if __name__ == "__main__":
    unittest.main()
