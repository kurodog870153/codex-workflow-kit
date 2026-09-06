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


class ExecuteCliTests(unittest.TestCase):
    def common_arguments(self) -> list[str]:
        return [
            "--project-root",
            "/project",
            "execute",
        ]

    def execute_scope_arguments(self) -> list[str]:
        return [
            "--user-config-root",
            "/config",
            "--task-path",
            "outputs/work/tasks/example.md",
            "--execution-dir",
            "outputs/work/executions/example",
            "--task-id",
            "TASK-001",
            "--skill-root",
            "repo:.agents/skills=/repo-skills",
            "--skill-root",
            "user:skills=/user-skills",
        ]

    def test_preflight_arguments_parse_repeated_inputs(self) -> None:
        arguments = build_parser().parse_args(
            self.common_arguments()
            + ["preflight"]
            + self.execute_scope_arguments()
            + [
                "--confirmed-input",
                "INPUT-001",
                "--confirmed-input",
                "INPUT-002",
            ]
        )

        self.assertEqual(arguments.command, "execute")
        self.assertEqual(arguments.execute_command, "preflight")
        self.assertEqual(arguments.project_root, "/project")
        self.assertEqual(arguments.user_config_root, "/config")
        self.assertEqual(
            arguments.skill_root,
            [
                "repo:.agents/skills=/repo-skills",
                "user:skills=/user-skills",
            ],
        )
        self.assertEqual(
            arguments.confirmed_input,
            ["INPUT-001", "INPUT-002"],
        )

    def test_attempt_start_arguments_require_stdin(self) -> None:
        arguments = build_parser().parse_args(
            self.common_arguments()
            + ["attempt-start"]
            + self.execute_scope_arguments()
            + ["--stdin"]
        )

        self.assertEqual(arguments.execute_command, "attempt-start")
        self.assertTrue(arguments.stdin)
        self.assertEqual(arguments.confirmed_input, [])

        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(
                self.common_arguments()
                + ["attempt-start"]
                + self.execute_scope_arguments()
            )

        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_record_begin_arguments_parse_record_id(self) -> None:
        arguments = build_parser().parse_args(
            self.common_arguments()
            + ["record-begin"]
            + self.execute_scope_arguments()
            + ["--record-id", "RECORD-001"]
        )

        self.assertEqual(arguments.execute_command, "record-begin")
        self.assertEqual(arguments.record_id, "RECORD-001")

    def test_preflight_missing_task_returns_domain_error(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "execute",
                    "preflight",
                    "--user-config-root",
                    project_directory,
                    "--task-path",
                    "outputs/work/tasks/missing.md",
                    "--execution-dir",
                    "outputs/work/executions/example",
                    "--task-id",
                    "TASK-001",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.IO_FAILURE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "execute_preflight_task_missing")


if __name__ == "__main__":
    unittest.main()
