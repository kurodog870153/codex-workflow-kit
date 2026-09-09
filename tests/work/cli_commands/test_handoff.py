from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import build_parser, main
from worklib.foundation.errors import ExitCode, WorkError
from worklib.skills.catalog import parse_skill_root


class HandoffCliTests(unittest.TestCase):
    def test_verify_task_requires_explicit_path_and_id(self):
        prefix = ["--project-root", "/project", "handoff", "verify-task-to-execute", "--stdin", "--user-config-root", "/config"]
        for options in ([], ["--task-path", "task.json"], ["--task-id", "TASK-001"]):
            with self.subTest(options=options), self.assertRaises(WorkError):
                build_parser().parse_args(prefix + options)

    def test_verify_task_dispatches_confirmed_target_and_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            incoming = {"marker": "WORK-HANDOFF"}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.cli_commands.handoff.verify_task_to_execute_handoff", return_value={"status": "valid"}) as verify:
                code = main(["--project-root", str(root), "handoff", "verify-task-to-execute", "--stdin",
                             "--task-path", "confirmed/example/task.json", "--task-id", "TASK-002",
                             "--user-config-root", str(root), "--skill-root", skill_root],
                            stdin=io.StringIO(json.dumps(incoming)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            verify.assert_called_once_with(root, incoming, task_path="confirmed/example/task.json", task_id="TASK-002",
                                           user_config_root=str(root), skill_roots=[parse_skill_root(skill_root)])

    def test_verify_plan_requires_source_options(self):
        prefix = ["--project-root", "/project", "handoff", "verify-plan-to-task"]
        for options in ([], ["--stdin"], ["--stdin", "--plan-path", "plan.json"],
                        ["--stdin", "--user-config-root", "/config"]):
            with self.subTest(options=options), self.assertRaises(WorkError):
                build_parser().parse_args(prefix + options)

    def test_verify_plan_dispatches_confirmed_path_and_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            incoming = {"marker": "WORK-HANDOFF"}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.cli_commands.handoff.verify_plan_to_task_handoff", return_value={"status": "valid"}) as verify:
                code = main(["--project-root", str(root), "handoff", "verify-plan-to-task", "--stdin",
                             "--plan-path", "confirmed/plan.json", "--user-config-root", str(root), "--skill-root", skill_root],
                            stdin=io.StringIO(json.dumps(incoming)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            verify.assert_called_once_with(root, incoming, plan_path="confirmed/plan.json",
                                           user_config_root=str(root), skill_roots=[parse_skill_root(skill_root)])

    def test_execute_return_requires_explicit_context(self):
        for command in ("build-execute-to-task", "build-execute-to-plan"):
            args = ["--project-root", "/project", "handoff", command, "--stdin",
                    "--task-path", "task.json", "--task-id", "TASK-001", "--user-config-root", "/config"]
            with self.subTest(command=command), self.assertRaises(WorkError):
                build_parser().parse_args(args)
            parsed = build_parser().parse_args(args + ["--attempt-id", "ATTEMPT-001"])
            self.assertEqual(parsed.attempt_id, "ATTEMPT-001")
            parsed = build_parser().parse_args(args + ["--preflight"])
            self.assertTrue(parsed.preflight)
            self.assertIsNone(parsed.attempt_id)
            with self.subTest(command=command), self.assertRaises(WorkError):
                build_parser().parse_args(args + ["--preflight", "--attempt-id", "ATTEMPT-001"])

    def test_preflight_return_dispatches_target_and_skill_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            request = {"summary": "Return for clarification"}
            skill_root = "repo:skills=" + str(root / "skills")
            for target in ("task", "plan"):
                output, error = io.StringIO(), io.StringIO()
                with patch("worklib.cli_commands.handoff.build_preflight_return_handoff", return_value={"marker": "WORK-HANDOFF"}) as build:
                    code = main([
                        "--project-root", str(root), "handoff", f"build-execute-to-{target}", "--stdin", "--preflight",
                        "--task-path", "task.json", "--task-id", "TASK-002", "--user-config-root", str(root),
                        "--skill-root", skill_root,
                    ], stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=error)
                self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                build.assert_called_once_with(root, request, direction=f"execute_to_{target}", task_path="task.json",
                                              task_id="TASK-002", user_config_root=str(root), skill_roots=[parse_skill_root(skill_root)])

    def test_build_task_to_plan_dispatches_optional_target_and_roots(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            request = {"summary": "Return for review"}
            skill_root = "repo:skills=" + str(root / "skills")
            for task_id in (None, "TASK-002"):
                output, error = io.StringIO(), io.StringIO()
                extra = [] if task_id is None else ["--task-id", task_id]
                with patch("worklib.cli_commands.handoff.build_task_to_plan_handoff", return_value={"marker": "WORK-HANDOFF"}) as build:
                    code = main([
                        "--project-root", str(root), "handoff", "build-task-to-plan", "--stdin",
                        "--task-path", "outputs/work/tasks/example/task.json", "--user-config-root", str(root),
                        "--skill-root", skill_root, *extra,
                    ], stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=error)
                self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                build.assert_called_once_with(root, request, task_path="outputs/work/tasks/example/task.json", task_id=task_id,
                                              user_config_root=str(root), skill_roots=[parse_skill_root(skill_root)])

    def test_return_builder_requires_formal_task_path(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(["--project-root", "/project", "handoff", "build-task-to-plan", "--stdin", "--user-config-root", "/config"])
        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_build_task_to_execute_dispatches_explicit_target(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            request = {"summary": "Execute reviewed task"}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.cli_commands.handoff.build_task_to_execute_handoff", return_value={"marker": "WORK-HANDOFF"}) as build:
                code = main([
                    "--project-root", str(root), "handoff", "build-task-to-execute", "--stdin",
                    "--task-path", "outputs/work/tasks/example/task.json", "--task-id", "TASK-002",
                    "--user-config-root", str(root), "--skill-root", skill_root,
                ], stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            build.assert_called_once_with(root, request, task_path="outputs/work/tasks/example/task.json", task_id="TASK-002",
                                          user_config_root=str(root), skill_roots=[parse_skill_root(skill_root)])

    def test_execute_handoff_requires_task_id_and_task_path(self) -> None:
        for extra in ([], ["--task-path", "outputs/work/tasks/example/task.json"], ["--task-id", "TASK-001"]):
            with self.subTest(extra=extra), self.assertRaises(WorkError) as context:
                build_parser().parse_args(["--project-root", "/project", "handoff", "build-task-to-execute", "--stdin", "--user-config-root", "/config", *extra])
            self.assertEqual(context.exception.code, "cli_usage_error")

    def test_build_plan_to_task_dispatches_semantic_input_and_skill_roots(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            request = {"summary": "Continue planning", "affected_ids": ["GOAL-001"]}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.cli_commands.handoff.build_plan_to_task_handoff", return_value={"marker": "WORK-HANDOFF"}) as build:
                code = main([
                    "--project-root", str(root), "handoff", "build-plan-to-task", "--stdin",
                    "--plan-path", "outputs/work/plans/example.json", "--user-config-root", str(root),
                    "--skill-root", skill_root,
                ], stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            self.assertEqual(json.loads(output.getvalue()), {"marker": "WORK-HANDOFF"})
            build.assert_called_once_with(root, request, plan_path="outputs/work/plans/example.json",
                                          user_config_root=str(root), skill_roots=[parse_skill_root(skill_root)])

    def test_build_requires_stdin_and_source_arguments(self) -> None:
        for extra in ([], ["--stdin"], ["--stdin", "--plan-path", "plan.json"], ["--plan-path", "plan.json", "--user-config-root", "/config"]):
            with self.subTest(extra=extra), self.assertRaises(WorkError) as context:
                build_parser().parse_args(["--project-root", "/project", "handoff", "build-plan-to-task", *extra])
            self.assertEqual(context.exception.code, "cli_usage_error")

    def test_build_rejects_invalid_json_before_reading_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output, error = io.StringIO(), io.StringIO()
            code = main([
                "--project-root", temporary, "handoff", "build-plan-to-task", "--stdin",
                "--plan-path", "outputs/work/plans/example.json", "--user-config-root", temporary,
            ], stdin=io.StringIO("{"), stdout=output, stderr=error)
            self.assertEqual(code, ExitCode.INPUT_FORMAT)
            self.assertEqual(json.loads(error.getvalue())["code"], "invalid_json_contract")
            self.assertEqual(output.getvalue(), "")

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
