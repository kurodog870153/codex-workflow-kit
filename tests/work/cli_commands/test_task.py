from __future__ import annotations

import io
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import build_parser, main
from worklib.foundation.errors import ExitCode


class TaskCliTests(unittest.TestCase):
    def test_source_update_commands_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command in ("draft-source-update", "draft-source-recover"):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    request = {"reason": "Reviewed", "selections": {"TASK-001": {"selected_paths": [], "references": []}}}
                    with patch("worklib.cli_commands.task.update_task_draft_sources", return_value={"status": "saved"}) as operation:
                        code = main([
                            "--project-root", str(root), "task", command, "--stdin",
                            "--requirement-id", "example", "--expected-revision", "2",
                            "--plan-path", "outputs/work/plans/example.md", "--user-config-root", str(root),
                        ], stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=error)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    operation.assert_called_once_with(root, "example", request, expected_revision=2,
                        plan_path="outputs/work/plans/example.md", user_config_root=str(root), skill_roots=[],
                        recover=command == "draft-source-recover")

    def test_assembly_commands_dispatch_metadata_and_approval(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command, name in (("draft-assemble", "assemble_task_drafts"), ("draft-create", "create_task_from_drafts")):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    arguments = ["--project-root", str(root), "task", command, "--stdin", "--requirement-id", "example", "--expected-revision", "2", "--plan-path", "outputs/work/plans/example.md", "--user-config-root", str(root)]
                    extra = {}
                    if command == "draft-create":
                        arguments += ["--approved-sha256", "a" * 64]
                        extra["approved_sha256"] = "a" * 64
                    metadata = {"title": "TASK", "summary": "Result"}
                    with patch("worklib.cli_commands.task." + name, return_value={"status": "valid"}) as operation:
                        code = main(arguments, stdin=io.StringIO(json.dumps(metadata)), stdout=output, stderr=error)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    operation.assert_called_once_with(root, "example", metadata, expected_revision=2, plan_path="outputs/work/plans/example.md", user_config_root=str(root), skill_roots=[], **extra)
    def test_list_update_and_recovery_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command in ("draft-list-update", "draft-list-recover"):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    with patch("worklib.cli_commands.task.update_task_planning_list", return_value={"status": "saved"}) as operation:
                        code = main([
                            "--project-root", str(root), "task", command,
                            "--stdin", "--expected-revision", "3",
                        ], stdin=io.StringIO(json.dumps({"index": {"revision": 4}, "reason": "Split task"})), stdout=output, stderr=error)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    operation.assert_called_once_with(root, {"revision": 4}, expected_revision=3, reason="Split task", recover=command == "draft-list-recover")

    def test_draft_check_dispatches_confirmed_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for selection, expected_paths in (
                (["--general-only"], []),
                (["--instruction-path", "web/backend"], ["web/backend"]),
            ):
                with self.subTest(selection=selection):
                    stdout, stderr = io.StringIO(), io.StringIO()
                    result = {"schema": "work-task-draft-source-check/v1", "status": "valid"}
                    with patch("worklib.cli_commands.task.check_task_draft_sources", return_value=result) as check:
                        code = main([
                            "--project-root", str(root), "task", "draft-check",
                            "--requirement-id", "example", "--task-id", "TASK-001",
                            "--expected-revision", "2", "--plan-path", "outputs/work/plans/example.md",
                            "--user-config-root", str(root), "--reference", "task.general.task-records",
                            *selection,
                        ], stdout=stdout, stderr=stderr)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    self.assertEqual(stderr.getvalue(), "")
                    self.assertEqual(json.loads(stdout.getvalue()), result)
                    check.assert_called_once_with(
                        root, "example", "TASK-001", expected_revision=2,
                        plan_path="outputs/work/plans/example.md", user_config_root=str(root),
                        skill_roots=[], selected_paths=expected_paths,
                        reference_names=["task.general.task-records"],
                    )

    def test_draft_check_requires_explicit_instruction_selection(self) -> None:
        with self.assertRaises(Exception) as context:
            build_parser().parse_args([
                "--project-root", "/project", "task", "draft-check",
                "--requirement-id", "example", "--task-id", "TASK-001",
                "--expected-revision", "1", "--plan-path", "outputs/work/plans/example.md",
                "--user-config-root", "/config",
            ])
        self.assertEqual(context.exception.code, "cli_usage_error")

    def test_create_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "task",
                "create",
                "--user-config-root",
                "/config",
                "--skill-root",
                "repo:.agents/skills=/skills",
                "--stdin",
                "--plan-path",
                "outputs/work/plans/example.md",
                "--task-path",
                "outputs/work/tasks/example/task.md",
                "--execution-dir",
                "outputs/work/executions/example",
            ]
        )

        self.assertEqual(arguments.command, "task")
        self.assertEqual(arguments.task_command, "create")
        self.assertEqual(arguments.project_root, "/project")
        self.assertEqual(arguments.user_config_root, "/config")
        self.assertEqual(
            arguments.skill_root,
            ["repo:.agents/skills=/skills"],
        )
        self.assertTrue(arguments.stdin)
        self.assertEqual(
            arguments.plan_path,
            "outputs/work/plans/example.md",
        )
        self.assertEqual(
            arguments.task_path,
            "outputs/work/tasks/example/task.md",
        )
        self.assertEqual(
            arguments.execution_dir,
            "outputs/work/executions/example",
        )

    def test_validate_stdin_requires_task_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "task",
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
        self.assertEqual(error["code"], "task_path_required")

    def test_validate_file_rejects_task_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "task",
                    "validate",
                    "--user-config-root",
                    project_directory,
                    "--path",
                    "outputs/work/tasks/example/task.md",
                    "--task-path",
                    "outputs/work/tasks/other/task.md",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "unexpected_task_path")


class TaskDraftCliTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.index = {
            "schema": "work-task-planning-index/v1", "requirement_id": "example",
            "revision": 1, "current_task_id": "TASK-001",
            "source": {"plan_sha256": "a" * 64, "hierarchy_selection_sha256": "b" * 64, "skill_selection_sha256": "c" * 64},
            "tasks": [{
                "id": "TASK-001", "title": "Source", "goal": "Update source.",
                "scope": ["Source only."], "skill_id": None, "dependencies": [],
                "status": "planned", "boundary_revision": 1, "instructions_sha256": "d" * 64,
            }],
        }
        self.draft = {
            "schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001",
            "revision": 1, "boundary_revision": 1, "source": copy.deepcopy(self.index["source"]),
            "instructions_sha256": "d" * 64, "status": "in_progress", "notes": ["討論中"],
            "confirmed_decisions": [], "tentative": [], "open_questions": ["Which test?"],
            "next_discussion_point": "Confirm test.",
        }

    def invoke(self, arguments, payload=None, raw=None):
        stdout, stderr = io.StringIO(), io.StringIO()
        code = main(
            ["--project-root", str(self.root), "task", *arguments],
            stdin=io.StringIO(raw if raw is not None else json.dumps(payload)),
            stdout=stdout, stderr=stderr,
        )
        return code, stdout.getvalue(), stderr.getvalue()

    def initialize(self):
        code, output, error = self.invoke(["draft-init", "--stdin"], self.index)
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output)["status"], "saved")

    def proposal(self):
        index = copy.deepcopy(self.index)
        index["revision"] = 2
        index["tasks"][0]["status"] = "in_progress"
        return {"index": index, "draft": self.draft}

    def test_init_save_and_selective_read_through_cli(self) -> None:
        self.initialize()
        code, output, error = self.invoke(["draft-read", "--requirement-id", "example"])
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output), self.index)
        code, output, error = self.invoke(["draft-save", "--stdin", "--expected-revision", "1"], self.proposal())
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output)["revision"], 2)
        code, output, error = self.invoke(["draft-read", "--requirement-id", "example", "--task-id", "TASK-001"])
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output), self.draft)
        self.assertFalse((self.root / "outputs/work/tasks/example/task.md").exists())

    def test_invalid_json_does_not_create_storage(self) -> None:
        code, output, error = self.invoke(["draft-init", "--stdin"], raw="{")
        self.assertEqual(code, ExitCode.INPUT_FORMAT)
        self.assertEqual(output, "")
        self.assertEqual(json.loads(error)["code"], "invalid_json_contract")
        self.assertFalse((self.root / "outputs").exists())

    def test_save_requires_both_objects_and_rejects_extra_fields(self) -> None:
        for payload in ({"index": self.index}, {"index": self.index, "draft": None}, {**self.proposal(), "approved": True}):
            with self.subTest(payload=payload):
                code, output, error = self.invoke(["draft-save", "--stdin", "--expected-revision", "0"], payload)
                self.assertEqual(code, ExitCode.CONTRACT)
                self.assertEqual(output, "")
                self.assertEqual(json.loads(error)["schema"], "work-error/v1")
        self.assertFalse((self.root / "outputs").exists())

    def test_stale_save_reports_conflict_without_overwrite(self) -> None:
        self.initialize()
        arguments = ["draft-save", "--stdin", "--expected-revision", "1"]
        self.assertEqual(self.invoke(arguments, self.proposal())[0], ExitCode.SUCCESS)
        path = self.root / "outputs/work/tasks/example/drafts/index.json"
        before = path.read_bytes()
        code, output, error = self.invoke(arguments, self.proposal())
        self.assertEqual(code, ExitCode.WORKFLOW_STATE)
        self.assertEqual(output, "")
        self.assertEqual(json.loads(error)["code"], "draft_revision_conflict")
        self.assertEqual(path.read_bytes(), before)

    def test_required_arguments_and_invalid_revision(self) -> None:
        for arguments in (["draft-init"], ["draft-save", "--stdin"], ["draft-save", "--stdin", "--expected-revision", "x"], ["draft-read"]):
            with self.subTest(arguments=arguments):
                code, output, error = self.invoke(arguments)
                self.assertEqual(code, ExitCode.CLI_USAGE)
                self.assertEqual(output, "")
                self.assertEqual(json.loads(error)["code"], "cli_usage_error")

    def test_missing_draft_read_is_read_only(self) -> None:
        code, output, error = self.invoke(["draft-read", "--requirement-id", "example"])
        self.assertEqual(code, ExitCode.IO_FAILURE)
        self.assertEqual(output, "")
        self.assertEqual(json.loads(error)["code"], "draft_read_failed")
        self.assertFalse((self.root / "outputs").exists())

    def test_recover_dispatches_initial_index_and_save_request(self) -> None:
        for revision, payload, expected_draft in ((0, self.index, None), (1, self.proposal(), self.draft)):
            with self.subTest(revision=revision):
                result = {"schema": "work-task-draft-recovery/v1", "status": "recovered"}
                with patch("worklib.cli_commands.task.recover_task_planning", return_value=result) as recover:
                    code, output, error = self.invoke(["draft-recover", "--stdin", "--expected-revision", str(revision)], payload)
                self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
                self.assertEqual(json.loads(output), result)
                if revision == 0:
                    recover.assert_called_once_with(self.root, self.index, expected_revision=0)
                else:
                    recover.assert_called_once_with(self.root, payload["index"], expected_revision=1, draft=expected_draft)


if __name__ == "__main__":
    unittest.main()
