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
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.cli import build_parser, main
from worklib.models.common.errors import ExitCode, WorkError
from worklib.services.skill_catalog import parse_skill_root
from worklib.orchestration.handoff import HandoffOperations


def operation_result(**payload):
    return {"schema": "work-test-operation-result/v1", **payload}


class HandoffCliTests(FileInputTestCase):
    def test_return_verifiers_dispatch_independent_targets_and_context(self):
        for direction, context in (("task-to-plan", []), ("execute-to-task", ["--preflight"]),
                                   ("execute-to-plan", ["--attempt-id", "ATTEMPT-001"])):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                output = io.StringIO()
                with patch("worklib.business_services.handoff.verify_return_handoff", return_value=operation_result(status="valid")) as verify:
                    code = main(self.input_arguments([
                        "--project-root", str(root), "handoff", "verify-" + direction,
                        "--input-file", "request.json", "--plan-path", "outputs/work/plans/example.json",
                        "--task-path", "outputs/work/tasks/example/task.json", "--task-id", "TASK-001",
                        "--user-config-root", str(root), *context,
                    ], "{}"), stdout=output, stderr=io.StringIO())
                self.assertEqual(code, 0, output.getvalue())
                self.assertEqual(verify.call_args.kwargs["direction"], direction.replace("-", "_"))
                self.assertEqual(verify.call_args.kwargs["preflight"], "--preflight" in context)
                self.assertEqual(verify.call_args.kwargs["attempt_id"], "ATTEMPT-001" if "--attempt-id" in context else None)

    def test_execute_return_verifier_requires_explicit_context(self):
        with self.assertRaises(WorkError) as caught:
            build_parser().parse_args(["handoff", "verify-execute-to-task", "--input-file", "request.json",
                "--plan-path", "outputs/work/plans/example.json", "--task-path", "outputs/work/tasks/example/task.json",
                "--task-id", "TASK-001", "--user-config-root", "/config"])
        self.assertEqual(caught.exception.code, "cli_usage_error")

    def test_verify_task_requires_explicit_path_and_id(self):
        prefix = ["--project-root", "/project", "handoff", "verify-task-to-execute", "--input-file", "request.json", "--user-config-root", "/config"]
        for options in ([], ["--task-path", "task.json"], ["--task-id", "TASK-001"]):
            with self.subTest(options=options), self.assertRaises(WorkError):
                build_parser().parse_args(prefix + options)

    def test_verify_task_dispatches_confirmed_target_and_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            incoming = {"marker": "WORK-HANDOFF"}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.business_services.handoff.verify_task_to_execute_handoff", return_value=operation_result(status="valid")) as verify:
                code = main(self.input_arguments(["--project-root", str(root), "handoff", "verify-task-to-execute", "--input-file", "request.json",
                             "--task-path", "confirmed/example/task.json", "--task-id", "TASK-002",
                             "--user-config-root", str(root), "--skill-root", skill_root], json.dumps(incoming)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            verify.assert_called_once_with(root, incoming, task_path="confirmed/example/task.json", task_id="TASK-002",
                                           user_config_root=str(root), operations=HandoffOperations,
                                           skill_roots=[parse_skill_root(skill_root)])

    def test_verify_plan_requires_source_options(self):
        prefix = ["--project-root", "/project", "handoff", "verify-plan-to-task"]
        for options in ([], ["--input-file", "request.json"], ["--input-file", "request.json", "--plan-path", "plan.json"],
                        ["--input-file", "request.json", "--user-config-root", "/config"]):
            with self.subTest(options=options), self.assertRaises(WorkError):
                build_parser().parse_args(prefix + options)

    def test_verify_plan_dispatches_confirmed_path_and_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            incoming = {"marker": "WORK-HANDOFF"}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.business_services.handoff.verify_plan_to_task_handoff", return_value=operation_result(status="valid")) as verify:
                code = main(self.input_arguments(["--project-root", str(root), "handoff", "verify-plan-to-task", "--input-file", "request.json",
                             "--plan-path", "confirmed/plan.json", "--user-config-root", str(root), "--skill-root", skill_root], json.dumps(incoming)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            verify.assert_called_once_with(root, incoming, plan_path="confirmed/plan.json",
                                           user_config_root=str(root), operations=HandoffOperations,
                                           skill_roots=[parse_skill_root(skill_root)])

    def test_execute_return_requires_explicit_context(self):
        for command in ("build-execute-to-task", "build-execute-to-plan"):
            args = ["--project-root", "/project", "handoff", command, "--input-file", "request.json",
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
                with patch("worklib.business_services.handoff.build_preflight_return_handoff", return_value=operation_result(marker="WORK-HANDOFF")) as build:
                    code = main(self.input_arguments([
                        "--project-root", str(root), "handoff", f"build-execute-to-{target}", "--input-file", "request.json", "--preflight",
                        "--task-path", "task.json", "--task-id", "TASK-002", "--user-config-root", str(root),
                        "--skill-root", skill_root,
                    ], json.dumps(request)), stdout=output, stderr=error)
                self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                build.assert_called_once_with(root, request, direction=f"execute_to_{target}", task_path="task.json",
                                              task_id="TASK-002", user_config_root=str(root), operations=HandoffOperations,
                                              skill_roots=[parse_skill_root(skill_root)])

    def test_build_task_to_plan_dispatches_optional_target_and_roots(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            request = {"summary": "Return for review"}
            skill_root = "repo:skills=" + str(root / "skills")
            for task_id in (None, "TASK-002"):
                output, error = io.StringIO(), io.StringIO()
                extra = [] if task_id is None else ["--task-id", task_id]
                with patch("worklib.business_services.handoff.build_task_to_plan_handoff", return_value=operation_result(marker="WORK-HANDOFF")) as build:
                    code = main(self.input_arguments([
                        "--project-root", str(root), "handoff", "build-task-to-plan", "--input-file", "request.json",
                        "--task-path", "outputs/work/tasks/example/task.json", "--user-config-root", str(root),
                        "--skill-root", skill_root, *extra,
                    ], json.dumps(request)), stdout=output, stderr=error)
                self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                build.assert_called_once_with(root, request, task_path="outputs/work/tasks/example/task.json", task_id=task_id,
                                              user_config_root=str(root), operations=HandoffOperations,
                                              skill_roots=[parse_skill_root(skill_root)])

    def test_return_builder_requires_formal_task_path(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_parser().parse_args(["--project-root", "/project", "handoff", "build-task-to-plan", "--input-file", "request.json", "--user-config-root", "/config"])
        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_build_task_to_execute_dispatches_explicit_target(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            request = {"summary": "Execute reviewed task"}
            skill_root = "repo:skills=" + str(root / "skills")
            with patch("worklib.business_services.handoff.build_task_to_execute_handoff", return_value=operation_result(marker="WORK-HANDOFF")) as build:
                code = main(self.input_arguments([
                    "--project-root", str(root), "handoff", "build-task-to-execute", "--input-file", "request.json",
                    "--task-path", "outputs/work/tasks/example/task.json", "--task-id", "TASK-002",
                    "--user-config-root", str(root), "--skill-root", skill_root,
                ], json.dumps(request)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            build.assert_called_once_with(root, request, task_path="outputs/work/tasks/example/task.json", task_id="TASK-002",
                                          user_config_root=str(root), operations=HandoffOperations,
                                          skill_roots=[parse_skill_root(skill_root)])

    def test_execute_handoff_requires_task_id_and_task_path(self) -> None:
        for extra in ([], ["--task-path", "outputs/work/tasks/example/task.json"], ["--task-id", "TASK-001"]):
            with self.subTest(extra=extra), self.assertRaises(WorkError) as context:
                build_parser().parse_args(["--project-root", "/project", "handoff", "build-task-to-execute", "--input-file", "request.json", "--user-config-root", "/config", *extra])
            self.assertEqual(context.exception.code, "cli_usage_error")

    def test_build_plan_to_task_dispatches_semantic_input_and_skill_roots(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output, error = io.StringIO(), io.StringIO()
            request = {"summary": "Continue planning", "affected_ids": ["GOAL-001"]}
            skill_root = "repo:skills=" + str(root / "skills")
            result = operation_result(marker="WORK-HANDOFF")
            with patch("worklib.business_services.handoff.build_plan_to_task_handoff", return_value=result) as build:
                code = main(self.input_arguments([
                    "--project-root", str(root), "handoff", "build-plan-to-task", "--input-file", "request.json",
                    "--plan-path", "outputs/work/plans/example.json", "--user-config-root", str(root),
                    "--skill-root", skill_root,
                ], json.dumps(request)), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
            self.assertEqual(json.loads(output.getvalue())["data"], result)
            build.assert_called_once_with(root, request, plan_path="outputs/work/plans/example.json",
                                          user_config_root=str(root), operations=HandoffOperations,
                                          skill_roots=[parse_skill_root(skill_root)])

    def test_build_requires_input_file_and_source_arguments(self) -> None:
        for extra in ([], ["--input-file", "request.json"], ["--input-file", "request.json", "--plan-path", "plan.json"], ["--plan-path", "plan.json", "--user-config-root", "/config"]):
            with self.subTest(extra=extra), self.assertRaises(WorkError) as context:
                build_parser().parse_args(["--project-root", "/project", "handoff", "build-plan-to-task", *extra])
            self.assertEqual(context.exception.code, "cli_usage_error")

    def test_build_rejects_invalid_json_before_reading_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output, error = io.StringIO(), io.StringIO()
            code = main(self.input_arguments([
                "--project-root", temporary, "handoff", "build-plan-to-task", "--input-file", "request.json",
                "--plan-path", "outputs/work/plans/example.json", "--user-config-root", temporary,
            ], "{"), stdout=output, stderr=error)
            self.assertEqual(code, ExitCode.INPUT_FORMAT)
            self.assertEqual(json.loads(output.getvalue())["reason_code"], "invalid_json_contract")
            self.assertEqual(error.getvalue(), "")

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
