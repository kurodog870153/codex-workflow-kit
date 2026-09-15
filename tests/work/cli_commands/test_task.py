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
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.cli import build_parser, main
from worklib.cli_commands.task import _specification_summary
from worklib.foundation.errors import ExitCode


class TaskCliTests(FileInputTestCase):
    def test_specification_summary_omits_complete_candidates(self):
        preview = {
            "schema": "work-spec-update/v1", "status": "valid", "record_id": "SPEC-UPDATE-002",
            "approved_sha256": "a" * 64, "affected_task_ids": ["TASK-001"],
            "changed_fields": ["/tasks/TASK-001/goal"],
            "file_readiness": "requires_execute_preflight", "candidate": {"plan": {}, "task": {}, "index": {}},
            "migration": {"instruction_review": "large"},
        }
        summary = _specification_summary({"schema": "work-spec-prepare/v1", "request": {},
                                          "preview": preview, "output_file": "prepared.json",
                                          "transport": {"request_field": "request"},
                                          "next_step": {"command": "task spec-validate", "input": "request"}})
        self.assertEqual(summary, {
            "schema": "work-specification-summary/v1", "status": "valid",
            "record_id": "SPEC-UPDATE-002", "approved_sha256": "a" * 64,
            "affected_task_ids": ["TASK-001"],
            "changed_fields": ["/tasks/TASK-001/goal"],
            "file_readiness": "requires_execute_preflight", "output_file": "prepared.json",
            "transport": {"request_field": "request"},
            "next_step": {"command": "task spec-validate", "input": "request"},
        })

    def test_published_migration_summary_retains_only_verification_transport(self):
        result = {
            "schema": "work-spec-update/v1", "status": "updated", "record_id": "SPEC-UPDATE-002",
            "approved_sha256": "a" * 64, "affected_task_ids": ["TASK-001"],
            "changed_fields": ["/tasks/TASK-001/goal"], "file_readiness": "requires_execute_preflight",
            "artifacts": {}, "candidate": {"large": True}, "migration": {"large": True},
            "verification_request": {"schema": "work-migration-verify-request/v1"},
            "next_step": {"command": "task migrate-verify", "input": "verification_request"},
        }
        summary = _specification_summary(result)
        self.assertEqual(summary["verification_request"], result["verification_request"])
        self.assertEqual(summary["next_step"], result["next_step"])
        self.assertNotIn("candidate", summary)
        self.assertNotIn("migration", summary)

    def test_summary_flag_is_limited_to_requested_specification_commands(self):
        supported = ("spec-prepare", "migrate-prepare", "spec-validate", "spec-update",
                     "migrate-validate", "migrate")
        for command in supported:
            with self.subTest(command=command):
                arguments = ["--project-root", "/project", "task", command,
                             "--input-file", "request.json", "--user-config-root", "/config", "--summary"]
                if command in {"spec-update", "migrate"}:
                    arguments.extend(["--approved-sha256", "a" * 64])
                self.assertTrue(build_parser().parse_args(arguments).summary)
        for command in ("repair-prepare", "spec-recover", "migrate-recover"):
            with self.subTest(command=command), self.assertRaises(Exception):
                build_parser().parse_args([
                    "--project-root", "/project", "task", command,
                    "--input-file", "request.json", "--user-config-root", "/config", "--summary",
                ])

    def test_spec_verify_arguments_parse(self):
        arguments = build_parser().parse_args([
            "--project-root", "/project", "task", "spec-verify",
            "--input-file", "request.json", "--user-config-root", "/config",
        ])
        self.assertEqual(arguments.task_command, "spec-verify")
        self.assertEqual(arguments.input_file, "request.json")

    def test_draft_preparation_commands_use_file_transport(self):
        from artifacts import test_task_draft_prepare as fixtures
        fixture = fixtures.DraftPreparationTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        common = ["--project-root", str(fixture.root), "task"]
        source = ["--input-file", "request.json", "--requirement-id", "example",
                  "--plan-path", fixture.options["plan_path"], "--user-config-root", str(fixture.root)]
        output = io.StringIO()
        code = main(self.input_arguments(common + ["draft-init-request", "--prepare-only"] + source,
                    json.dumps(fixture.request)), stdout=output, stderr=io.StringIO())
        self.assertEqual(code, 0, output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["data"]["status"], "prepared")
        fixture.initialize()
        changed = copy.deepcopy(fixture.boundary)
        changed["goal"] = "Changed"
        payload = {"upsert": [changed], "remove_task_ids": [], "current_task_id": None, "reason": "Confirmed"}
        output = io.StringIO()
        code = main(self.input_arguments(common + ["draft-list-prepare", "--expected-revision", "1"] + source,
                    json.dumps(payload)), stdout=output, stderr=io.StringIO())
        self.assertEqual(code, 0, output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["data"]["affected_task_ids"], ["TASK-001"])

    def test_draft_status_dispatches_optional_explicit_task(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for task_id in (None, "TASK-002"):
                with self.subTest(task_id=task_id):
                    output, error = io.StringIO(), io.StringIO()
                    extra = [] if task_id is None else ["--task-id", task_id]
                    with patch("worklib.cli_commands.task.task_draft_status", return_value={"next_action": "confirm_start"}) as operation:
                        code = main(["--project-root", str(root), "task", "draft-status", "--requirement-id", "example", *extra], stdout=output, stderr=error)
                    self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                    self.assertEqual(json.loads(output.getvalue())["data"]["next_action"], "confirm_start")
                    operation.assert_called_once_with(root, "example", task_id=task_id)

    def test_single_draft_request_commands_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command in ("draft-save-request", "draft-recover-request"):
                for selection, paths in ((["--general-only"], []), (["--instruction-path", "web/backend"], ["web/backend"])):
                    with self.subTest(command=command, selection=selection):
                        output, error = io.StringIO(), io.StringIO()
                        request = {"status": "in_progress", "notes": ["Discussion"]}
                        with patch("worklib.cli_commands.task.save_task_draft_request", return_value={"status": "saved"}) as operation:
                            code = main(self.input_arguments([
                                "--project-root", str(root), "task", command, "--input-file", "request.json",
                                "--requirement-id", "example", "--task-id", "TASK-001",
                                "--expected-revision", "2", "--plan-path", "outputs/work/plans/example.json",
                                "--user-config-root", str(root), "--reference", "task.general.task-records",
                                *selection,
                            ], json.dumps(request)), stdout=output, stderr=error)
                        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                        operation.assert_called_once_with(
                            root, "example", "TASK-001", request, expected_revision=2,
                            plan_path="outputs/work/plans/example.json", user_config_root=str(root),
                            skill_roots=[], selected_paths=paths, reference_names=["task.general.task-records"],
                            recover=command == "draft-recover-request",
                        )

    def test_single_draft_request_requires_input_file_and_rejects_conflicting_selection(self) -> None:
        for command in ("draft-save-request", "draft-recover-request"):
            for extra in ([], ["--general-only"], ["--input-file", "request.json", "--general-only", "--instruction-path", "web"]):
                with self.subTest(command=command, extra=extra):
                    with self.assertRaises(Exception) as context:
                        build_parser().parse_args([
                            "--project-root", "/project", "task", command,
                            "--requirement-id", "example", "--task-id", "TASK-001",
                            "--expected-revision", "1", "--plan-path", "outputs/work/plans/example.json",
                            "--user-config-root", "/config", *extra,
                        ])
                    self.assertEqual(context.exception.code, "cli_usage_error")

    def test_source_update_commands_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command in ("draft-source-update", "draft-source-recover"):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    request = {"reason": "Reviewed", "selections": {"TASK-001": {"selected_paths": [], "references": []}}}
                    with patch("worklib.cli_commands.task.update_task_draft_sources", return_value={"status": "saved"}) as operation:
                        code = main(self.input_arguments([
                            "--project-root", str(root), "task", command, "--input-file", "request.json",
                            "--requirement-id", "example", "--expected-revision", "2",
                            "--plan-path", "outputs/work/plans/example.json", "--user-config-root", str(root),
                        ], json.dumps(request)), stdout=output, stderr=error)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    operation.assert_called_once_with(root, "example", request, expected_revision=2,
                        plan_path="outputs/work/plans/example.json", user_config_root=str(root), skill_roots=[],
                        recover=command == "draft-source-recover")

    def test_assembly_commands_dispatch_metadata_and_approval(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command, name in (("draft-assemble", "assemble_task_drafts"), ("draft-create", "create_task_from_drafts")):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    arguments = ["--project-root", str(root), "task", command, "--input-file", "request.json", "--requirement-id", "example", "--expected-revision", "2", "--plan-path", "outputs/work/plans/example.json", "--user-config-root", str(root)]
                    extra = {}
                    if command == "draft-create":
                        arguments += ["--approved-sha256", "a" * 64]
                        extra["approved_sha256"] = "a" * 64
                    metadata = {"title": "TASK", "summary": "Result"}
                    with patch("worklib.cli_commands.task." + name, return_value={"status": "valid"}) as operation:
                        code = main(self.input_arguments(arguments, json.dumps(metadata)), stdout=output, stderr=error)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    operation.assert_called_once_with(root, "example", metadata, expected_revision=2, plan_path="outputs/work/plans/example.json", user_config_root=str(root), skill_roots=[], **extra)
    def test_list_update_and_recovery_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command in ("draft-list-update", "draft-list-recover"):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    with patch("worklib.cli_commands.task.update_task_planning_list", return_value={"status": "saved"}) as operation:
                        code = main(self.input_arguments([
                            "--project-root", str(root), "task", command,
                            "--input-file", "request.json", "--expected-revision", "3",
                        ], json.dumps({"index": {"revision": 4}, "reason": "Split task"})), stdout=output, stderr=error)
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
                            "--expected-revision", "2", "--plan-path", "outputs/work/plans/example.json",
                            "--user-config-root", str(root), "--reference", "task.general.task-records",
                            *selection,
                        ], stdout=stdout, stderr=stderr)
                    self.assertEqual(code, ExitCode.SUCCESS)
                    self.assertEqual(stderr.getvalue(), "")
                    self.assertEqual(json.loads(stdout.getvalue())["data"], result)
                    check.assert_called_once_with(
                        root, "example", "TASK-001", expected_revision=2,
                        plan_path="outputs/work/plans/example.json", user_config_root=str(root),
                        skill_roots=[], selected_paths=expected_paths,
                        reference_names=["task.general.task-records"],
                    )

    def test_draft_commands_dispatch_missing_flags_as_saved_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for command, function in (("draft-check", "check_task_draft_sources"), ("draft-save-request", "save_task_draft_request"), ("draft-recover-request", "save_task_draft_request")):
                with self.subTest(command=command):
                    output, error = io.StringIO(), io.StringIO()
                    extra = [] if command == "draft-check" else ["--input-file", "request.json"]
                    with patch("worklib.cli_commands.task." + function, return_value={"status": "valid"}) as operation:
                        code = main(self.input_arguments([
                            "--project-root", str(root), "task", command,
                            "--requirement-id", "example", "--task-id", "TASK-001",
                            "--expected-revision", "1", "--plan-path", "outputs/work/plans/example.json",
                            "--user-config-root", str(root), *extra,
                        ], "{}"), stdout=output, stderr=error)
                    self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
                    self.assertIsNone(operation.call_args.kwargs["selected_paths"])
                    self.assertIsNone(operation.call_args.kwargs["reference_names"])

    def test_draft_commands_reject_references_without_explicit_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            for command in ("draft-check", "draft-save-request", "draft-recover-request"):
                output, error = io.StringIO(), io.StringIO()
                extra = [] if command == "draft-check" else ["--input-file", "request.json"]
                code = main(self.input_arguments([
                    "--project-root", temporary, "task", command,
                    "--requirement-id", "example", "--task-id", "TASK-001",
                    "--expected-revision", "1", "--plan-path", "outputs/work/plans/example.json",
                    "--user-config-root", temporary, "--reference", "task.general.task-records", *extra,
                ], "{}"), stdout=output, stderr=error)
                self.assertEqual(code, ExitCode.CLI_USAGE)
                self.assertEqual(json.loads(output.getvalue())["reason_code"], "draft_selection_incomplete")

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
                "--input-file", "request.json",
                "--plan-path",
                "outputs/work/plans/example.json",
                "--task-path",
                "outputs/work/tasks/example/task.json",
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
        self.assertTrue(arguments.input_file)
        self.assertEqual(
            arguments.plan_path,
            "outputs/work/plans/example.json",
        )
        self.assertEqual(
            arguments.task_path,
            "outputs/work/tasks/example/task.json",
        )
        self.assertEqual(
            arguments.execution_dir,
            "outputs/work/executions/example",
        )

    def test_validate_input_file_requires_task_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                self.input_arguments([
                    "--project-root",
                    project_directory,
                    "task",
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
        self.assertEqual(error["reason_code"], "task_path_required")

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
                    "outputs/work/tasks/example/task.json",
                    "--task-path",
                    "outputs/work/tasks/other/task.json",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stderr.getvalue(), "")
        error = json.loads(stdout.getvalue())
        self.assertEqual(error["schema"], "work-cli-result/v1")
        self.assertEqual(error["reason_code"], "unexpected_task_path")


class TaskDraftCliTests(FileInputTestCase):
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
            self.input_arguments(["--project-root", str(self.root), "task", *arguments], raw if raw is not None else json.dumps(payload)),
            stdout=stdout, stderr=stderr,
        )
        return code, stdout.getvalue(), stderr.getvalue()

    def initialize(self):
        code, output, error = self.invoke(["draft-init", "--input-file", "request.json"], self.index)
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output)["data"]["status"], "saved")

    def proposal(self):
        index = copy.deepcopy(self.index)
        index["revision"] = 2
        index["tasks"][0]["status"] = "in_progress"
        return {"index": index, "draft": self.draft}

    def test_init_save_and_selective_read_through_cli(self) -> None:
        self.initialize()
        code, output, error = self.invoke(["draft-read", "--requirement-id", "example"])
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output)["data"], self.index)
        code, output, error = self.invoke(["draft-save", "--input-file", "request.json", "--expected-revision", "1"], self.proposal())
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output)["data"]["revision"], 2)
        code, output, error = self.invoke(["draft-read", "--requirement-id", "example", "--task-id", "TASK-001"])
        self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output)["data"], self.draft)
        self.assertFalse((self.root / "outputs/work/tasks/example/task.json").exists())

    def test_invalid_json_does_not_create_storage(self) -> None:
        code, output, error = self.invoke(["draft-init", "--input-file", "request.json"], raw="{")
        self.assertEqual(code, ExitCode.INPUT_FORMAT)
        self.assertEqual(error, "")
        self.assertEqual(json.loads(output)["reason_code"], "invalid_json_contract")
        self.assertFalse((self.root / "outputs").exists())

    def test_save_requires_both_objects_and_rejects_extra_fields(self) -> None:
        for payload in ({"index": self.index}, {"index": self.index, "draft": None}, {**self.proposal(), "approved": True}):
            with self.subTest(payload=payload):
                code, output, error = self.invoke(["draft-save", "--input-file", "request.json", "--expected-revision", "0"], payload)
                self.assertEqual(code, ExitCode.CONTRACT)
                self.assertEqual(error, "")
                self.assertEqual(json.loads(output)["schema"], "work-cli-result/v1")
        self.assertFalse((self.root / "outputs").exists())

    def test_stale_save_reports_conflict_without_overwrite(self) -> None:
        self.initialize()
        arguments = ["draft-save", "--input-file", "request.json", "--expected-revision", "1"]
        self.assertEqual(self.invoke(arguments, self.proposal())[0], ExitCode.SUCCESS)
        path = self.root / "outputs/work/tasks/example/drafts/index.json"
        before = path.read_bytes()
        code, output, error = self.invoke(arguments, self.proposal())
        self.assertEqual(code, ExitCode.WORKFLOW_STATE)
        self.assertEqual(error, "")
        self.assertEqual(json.loads(output)["reason_code"], "draft_revision_conflict")
        self.assertEqual(path.read_bytes(), before)

    def test_required_arguments_and_invalid_revision(self) -> None:
        for arguments in (["draft-init"], ["draft-save", "--input-file", "request.json"], ["draft-save", "--input-file", "request.json", "--expected-revision", "x"], ["draft-read"]):
            with self.subTest(arguments=arguments):
                code, output, error = self.invoke(arguments)
                self.assertEqual(code, ExitCode.CLI_USAGE)
                self.assertEqual(error, "")
                self.assertEqual(json.loads(output)["reason_code"], "cli_usage_error")

    def test_missing_draft_read_is_read_only(self) -> None:
        code, output, error = self.invoke(["draft-read", "--requirement-id", "example"])
        self.assertEqual(code, ExitCode.IO_FAILURE)
        self.assertEqual(error, "")
        self.assertEqual(json.loads(output)["reason_code"], "draft_read_failed")
        self.assertFalse((self.root / "outputs").exists())

    def test_recover_dispatches_initial_index_and_save_request(self) -> None:
        for revision, payload, expected_draft in ((0, self.index, None), (1, self.proposal(), self.draft)):
            with self.subTest(revision=revision):
                result = {"schema": "work-task-draft-recovery/v1", "status": "recovered"}
                with patch("worklib.cli_commands.task.recover_task_planning", return_value=result) as recover:
                    code, output, error = self.invoke(["draft-recover", "--input-file", "request.json", "--expected-revision", str(revision)], payload)
                self.assertEqual((code, error), (ExitCode.SUCCESS, ""))
                self.assertEqual(json.loads(output)["data"], result)
                if revision == 0:
                    recover.assert_called_once_with(self.root, self.index, expected_revision=0)
                else:
                    recover.assert_called_once_with(self.root, payload["index"], expected_revision=1, draft=expected_draft)


if __name__ == "__main__":
    unittest.main()
