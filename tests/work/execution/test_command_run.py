from __future__ import annotations

import copy
import io
import json
import platform
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills/work/scripts"))

from cli_support import FileInputTestCase
from contracts import test_task as fixtures
from worklib.cli import main
from worklib.services.attempt import render_attempt_contract
from worklib.services.attempt import authorization_sha256, minimal_authorization
from worklib.services.attempt import build_initial_execution_index, render_execution_index
from worklib.business_services.task.document import render_task_contract
from worklib.business_services.task.creation import prepare_task_collection_create
from worklib.business_services.execution import command_run
from worklib.business_services.execution.command_correction import record_command_correction
from worklib.business_services.execution.instructions import BASE_EXECUTE_REFERENCES
from worklib.orchestration.execution import ExecutionOperations
from worklib.orchestration.execution import begin_record, finish_record
from worklib.models.execution import ExecutionDeviationContract
from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.json_contract import parse_json_contract
from worklib.business_services.plan import render_plan_contract, validate_plan_contract
from worklib.business_services.instruction import build_instruction_selection
from worklib.technical.infrastructure import command_execution


class CommandRunTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.TaskInstructionContractTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.project_root
        task_artifact = str(Path(self.fixture.artifacts["task"]).with_name("index.json")).replace("\\", "/")
        self.fixture.artifacts["task"] = task_artifact
        self.fixture.contract["artifacts"]["task"] = task_artifact
        plan_path = self.root / self.fixture.artifacts["plan"]
        plan = parse_json_contract(plan_path.read_bytes(), source=str(plan_path))
        plan["artifacts"]["task"] = task_artifact
        plan_path.write_bytes(render_plan_contract(plan))
        plan_validation = validate_plan_contract(
            plan_path.read_bytes(),
            source=str(plan_path),
            actual_plan_path=self.fixture.artifacts["plan"],
            project_root=self.root,
            user_config_root=str(self.root),
            _allow_task_index=True,
        )
        self.fixture.contract["source_plan"]["canonical_sha256"] = plan_validation[
            "plan_sha256"
        ]
        self.task_path = self.root / task_artifact
        self.directory = self.root / self.fixture.artifacts["execution"]
        self.attempt_path = self.directory / "TASK-001/ATTEMPT-001/attempt.json"
        self.index_path = self.directory / "index.json"
        self.task_path.parent.mkdir(parents=True, exist_ok=True)
        self.attempt_path.parent.mkdir(parents=True)
        self.common = dict(project_root=self.root, user_config_root=str(self.root),
            raw_task_path=self.fixture.artifacts["task"], raw_execution_dir=self.fixture.artifacts["execution"],
            task_id="TASK-001", source="test")
        self.request = {"schema": "work-command-run-request/v1", "timeout_seconds": 10}
        self.configure([sys.executable, "-c", "print('observed')"])

    def configure(self, argv):
        self.fixture.contract["execution_defaults"] = {"working_directory": ".",
            "os": {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}[platform.system()], "shell": "sh"}
        self.fixture.contract["tasks"][0]["commands"][0] = {"id": "CMD-001", "mode": "argv", "argv": argv}
        bundle = prepare_task_collection_create(
            render_task_contract(self.fixture.contract),
            source="command-run fixture",
            raw_plan_path=self.fixture.artifacts["plan"],
            raw_task_path=self.fixture.artifacts["task"],
            project_root=self.root,
            user_config_root=str(self.root),
        )
        self.task_path.parent.mkdir(parents=True, exist_ok=True)
        self.task_path.write_bytes(bundle["index_raw"])
        item_directory = self.task_path.parent / "tasks"
        item_directory.mkdir(parents=True, exist_ok=True)
        for task_id, raw in bundle["items"].items():
            item_directory.joinpath(f"{task_id}.json").write_bytes(raw)
        validation = bundle["validation"]
        self.index = build_initial_execution_index(self.fixture.contract, validation)
        selection = build_instruction_selection(skill_root=fixtures.SKILL_ROOT, mode="execute",
            selected_paths=self.fixture.contract["tasks"][0]["instruction_selection"]["selected_paths"],
            reference_names=BASE_EXECUTE_REFERENCES)
        authorization = minimal_authorization()
        authorization["commands"] = [copy.deepcopy(self.fixture.contract["tasks"][0]["commands"][0])]
        authorization["working_directories"] = ["."]
        self.attempt = {"schema": "work-attempt/v1", "attempt_id": "ATTEMPT-001", "task_id": "TASK-001",
            "task_spec_id": self.index["task_spec_id"], "skill_id": None, "status": "in_progress",
            "task_collection_sha256": self.index["task_collection_sha256"],
            "task_index_sha256": self.index["task_index_sha256"],
            "task_item_sha256": self.index["tasks"][0]["task_item_sha256"],
            "task_instructions_sha256": self.index["tasks"][0]["instructions_sha256"],
            "execute_instructions_sha256": selection["instructions_sha256"],
            "hierarchy_selection_sha256": self.index["hierarchy_selection_sha256"],
            "execute_skill_selection_sha256": self.index["skill_selection_sha256"],
            "authorization": authorization,
            "authorization_sha256": authorization_sha256(authorization),
            "started_at": "2026-09-01T10:00+08:00", "records": []}
        self.index["tasks"][0].update(status="in_progress", latest_attempt="ATTEMPT-001")
        self.index["overall_status"] = "in_progress"
        self.index["lock"] = {"kind": "execution", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
            "record_id": "CMD-001", "execute_instructions_sha256": selection["instructions_sha256"]}
        self.save()

    def save(self):
        self.index_path.write_bytes(render_execution_index(self.index))
        self.attempt_path.write_bytes(render_attempt_contract(self.attempt, project_root=self.root))

    def prepare(self):
        return command_run.prepare_command(
            json.dumps(self.request).encode(), **self.common, operations=ExecutionOperations
        )

    def run_cmd(self, approval):
        return command_run.run_command(
            json.dumps(self.request).encode(), **self.common,
            approved_sha256=approval, operations=ExecutionOperations,
        )

    def snapshot(self):
        return {str(path): path.read_bytes() for path in self.root.rglob("*") if path.is_file()}

    def test_preview_is_read_only_and_literal_argv_runs_once(self):
        script = "import json,sys; from pathlib import Path; Path('observed.json').write_text(json.dumps(sys.argv[1:])); print('done')"
        arguments = ["two words", "$HOME", "$(touch unwanted)", "a;b", "x|y", 'a"b']
        self.configure([sys.executable, "-c", script, *arguments])
        before = self.snapshot()
        preview = self.prepare()
        self.assertEqual((preview["attempt_id"], preview["record_id"]), ("ATTEMPT-001", "CMD-001"))
        self.assertEqual(before, self.snapshot())
        result = self.run_cmd(preview["approved_sha256"])
        self.assertEqual(json.loads((self.root / "observed.json").read_text()), arguments)
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(self.attempt_path.read_bytes(), before[str(self.attempt_path)])
        self.assertEqual(self.index_path.read_bytes(), before[str(self.index_path)])
        with patch.object(command_run, "_execute") as execute:
            with self.assertRaises(WorkError) as error:
                self.run_cmd(preview["approved_sha256"])
            self.assertEqual(error.exception.code, "command_run_already_started")
            execute.assert_not_called()
        finish = finish_record(json.dumps(result["record_finish_request"]).encode(), **self.common)
        self.assertEqual(finish["lock_status"], "attempt_held")

    def test_wrong_or_stale_approval_never_launches(self):
        preview = self.prepare()
        with patch.object(command_run, "_execute") as execute:
            with self.assertRaises(WorkError):
                self.run_cmd("0" * 64)
            self.request["timeout_seconds"] = 11
            with self.assertRaises(WorkError) as error:
                self.run_cmd(preview["approved_sha256"])
            self.assertEqual(error.exception.code, "command_run_approval_changed")
            execute.assert_not_called()
        self.assertFalse(list(self.attempt_path.parent.glob(".work-command-*")))

    def test_saved_correction_is_the_only_actual_command(self):
        original = copy.deepcopy(self.fixture.contract["tasks"][0]["commands"][0])
        original.pop("id")
        self.index["lock"]["command_correction"] = {"original_command": original,
            "actual_command": {"mode": "argv", "argv": [sys.executable, "-c", "print('corrected')"]},
            "reason": "Reviewed equivalent test command", "authorization_evidence": "Confirmed test correction"}
        self.save()
        preview = self.prepare()
        self.assertEqual(preview["invocation"]["argv"][-1], "print('corrected')")
        self.assertIn("corrected", self.run_cmd(preview["approved_sha256"])["stdout_tail"])

    def test_correction_derives_identity_and_effective_original_command(self):
        replacement = {"mode": "argv", "argv": [sys.executable, "-c", "print('replacement')"]}
        corrected = {"mode": "argv", "argv": [sys.executable, "-c", "print('corrected')"]}
        action = {"kind": "replace_command", "record_id": "CMD-001", "replacement": corrected}
        deviation = copy.deepcopy(ExecutionDeviationContract.contract_example)
        deviation["proposal"]["action"] = {**action, "replacement": replacement}
        deviation["supplemental_authorization"]["action"] = copy.deepcopy(deviation["proposal"]["action"])
        self.attempt["execution_deviations"] = [deviation]
        self.attempt["authorization"]["allowed_deviations"] = [action]
        self.attempt["authorization_sha256"] = authorization_sha256(self.attempt["authorization"])
        self.save()
        request = {"schema": "work-command-correction-request/v1", "actual_command": corrected,
                   "reason": "Use the equivalent command."}
        result = record_command_correction(json.dumps(request).encode(), **self.common,
                                           operations=ExecutionOperations)
        self.assertEqual((result["attempt_id"], result["record_id"]), ("ATTEMPT-001", "CMD-001"))
        correction = json.loads(self.index_path.read_bytes())["lock"]["command_correction"]
        self.assertEqual(correction["original_command"], replacement)
        self.assertEqual(correction["actual_command"], corrected)

    def test_run_request_rejects_caller_identity(self):
        for field, value in (("attempt_id", "ATTEMPT-001"), ("record_id", "CMD-001")):
            with self.subTest(field=field):
                self.request[field] = value
                with self.assertRaises(WorkError) as error:
                    self.prepare()
                self.assertEqual(error.exception.code, "invalid_object_fields")
                self.request.pop(field)

    def test_supplemental_replacement_flows_from_record_begin_through_finish(self):
        replacement = {
            "kind": "replace_command",
            "record_id": "CMD-001",
            "replacement": {
                "mode": "argv",
                "argv": [sys.executable, "-c", "print('supplemental')"],
            },
        }
        deviation = copy.deepcopy(ExecutionDeviationContract.contract_example)
        deviation["proposal"]["action"] = copy.deepcopy(replacement)
        deviation["proposal"]["modifiable_files"] = ["src/supplemental.py"]
        deviation["supplemental_authorization"]["action"] = copy.deepcopy(replacement)
        deviation["supplemental_authorization"]["modifiable_files"] = [
            "src/supplemental.py"
        ]
        deviation["supplemental_authorization"]["authorization_evidence"] = (
            "User approved the supplemental command."
        )
        deviation["decision"]["evidence"] = "User approved the supplemental command."
        self.attempt["execution_deviations"] = [deviation]
        self.index["lock"].pop("record_id")
        self.save()

        begun = begin_record(
            project_root=self.root,
            user_config_root=str(self.root),
            raw_task_path=self.fixture.artifacts["task"],
            raw_execution_dir=self.fixture.artifacts["execution"],
            task_id="TASK-001",
            base_record_id="CMD-001",
        )
        self.assertEqual(begun["record_id"], "CMD-001")
        preview = self.prepare()
        self.assertEqual(preview["invocation"]["argv"][-1], "print('supplemental')")
        result = self.run_cmd(preview["approved_sha256"])
        started = json.loads(
            (self.root / (preview["receipt_prefix"] + ".started.json")).read_bytes()
        )
        self.assertEqual(
            started["authorization_evidence"],
            "User approved the supplemental command.",
        )
        self.assertIn("supplemental", result["stdout_tail"])
        result["record_finish_request"]["modified_files"] = ["src/supplemental.py"]
        self.assertEqual(
            finish_record(
                json.dumps(result["record_finish_request"]).encode(), **self.common
            )["record_status"],
            "recorded",
        )

    def test_windows_cmd_preview_is_read_only_and_executes_through_fixed_launcher(self):
        windows = self.root / "Windows"
        launcher = windows / "System32/cmd.exe"
        launcher.parent.mkdir(parents=True)
        launcher.write_bytes(b"fixed launcher")
        script = self.root / "tools/batch tool.cmd"
        script.parent.mkdir(parents=True)
        script.write_bytes(b"@echo off\r\n")
        arguments = ["two words", "a&b", "%PATH%", "!literal!", 'a"b']
        with patch.object(command_run.platform, "system", return_value="Windows"):
            self.configure([str(script), *arguments])
            before = self.snapshot()
            with patch.dict(command_run.os.environ, {"SystemRoot": str(windows)}):
                preview = self.prepare()
                self.assertEqual(before, self.snapshot())
                invocation = preview["invocation"]
                self.assertEqual(invocation["kind"], "windows_batch")
                self.assertEqual(invocation["launcher"], str(launcher.absolute()))
                self.assertEqual(invocation["script"], str(script.absolute()))
                self.assertEqual(invocation["arguments"], arguments)
                self.assertEqual(invocation["launcher_arguments"][:4], ["/d", "/s", "/v:off", "/c"])
                self.assertIn("%%PATH%%", invocation["command_line"])
                first = preview["approved_sha256"]
                script.write_bytes(b"@echo changed\r\n")
                self.assertNotEqual(first, self.prepare()["approved_sha256"])
                with patch.object(command_run, "_execute") as execute:
                    execute.return_value = {"status": "exited", "exit_code": 0,
                        "stdout_tail": "done", "stdout_truncated": False,
                        "stderr_tail": "", "stderr_truncated": False}
                    approved = self.prepare()
                    result = self.run_cmd(approved["approved_sha256"])
                    execute.assert_called_once_with(
                        [str(launcher.absolute()), *approved["invocation"]["launcher_arguments"]],
                        str(self.root), 10)
                    self.assertEqual(result["exit_code"], 0)
                    self.assertTrue((self.root / (approved["receipt_prefix"] + ".started.json")).is_file())
                    self.assertTrue((self.root / (approved["receipt_prefix"] + ".finished.json")).is_file())

    def test_windows_batch_resolves_path_and_non_windows_rejects(self):
        script = self.root / "tools/tool.bat"
        script.parent.mkdir(parents=True)
        script.write_bytes(b"@echo off\r\n")
        windows = self.root / "Windows"
        launcher = windows / "System32/cmd.exe"
        launcher.parent.mkdir(parents=True)
        launcher.write_bytes(b"fixed launcher")
        with patch.object(command_run.platform, "system", return_value="Windows"), \
             patch.object(command_run.shutil, "which", return_value=str(script)):
            self.configure(["tool.bat", "value"])
            with patch.dict(command_run.os.environ, {"SystemRoot": str(windows)}):
                preview = self.prepare()
                self.assertEqual(preview["invocation"]["script"], str(script.absolute()))
                with patch.object(command_run, "_execute", return_value={
                    "status": "exited", "exit_code": 0, "stdout_tail": "",
                    "stdout_truncated": False, "stderr_tail": "", "stderr_truncated": False,
                }) as execute:
                    self.run_cmd(preview["approved_sha256"])
                    execute.assert_called_once_with(
                        [str(launcher.absolute()), *preview["invocation"]["launcher_arguments"]],
                        str(self.root), 10)
        with patch.object(command_run.platform, "system", return_value="Linux"):
            self.configure([str(script)])
            with self.assertRaises(WorkError) as error:
                self.prepare()
        self.assertEqual(error.exception.code, "command_run_argv_only")

    def test_nonzero_exit_is_retained_without_retry(self):
        self.configure([sys.executable, "-c", "import sys; print('failed'); sys.exit(7)"])
        preview = self.prepare()
        with self.assertRaises(WorkError) as error:
            self.run_cmd(preview["approved_sha256"])
        self.assertEqual(error.exception.code, "command_run_failed")
        self.assertEqual(error.exception.details["exit_code"], 7)
        self.assertEqual(error.exception.details["record_finish_request"]["record"]["exit_code"], 7)
        self.assertEqual(json.loads(self.index_path.read_bytes())["lock"]["record_id"], "CMD-001")

    def test_timeout_keeps_receipts_and_never_invents_exit_code(self):
        preview = self.prepare()
        with patch.object(command_execution.subprocess, "run", side_effect=subprocess.TimeoutExpired("controlled", 10)) as run:
            with self.assertRaises(WorkError) as error:
                self.run_cmd(preview["approved_sha256"])
            run.assert_called_once()
        self.assertEqual(error.exception.details["status"], "timed_out")
        self.assertIsNone(error.exception.details["exit_code"])
        self.assertNotIn("record_finish_request", error.exception.details)
        self.assertEqual(len(list(self.attempt_path.parent.glob(".work-command-*"))), 2)

    def test_partial_receipt_blocks_execution(self):
        preview = self.prepare()
        (self.root / (preview["receipt_prefix"] + ".started.json")).write_bytes(b'{"partial":')
        with patch.object(command_run, "_execute") as execute:
            with self.assertRaises(WorkError):
                self.run_cmd(preview["approved_sha256"])
            execute.assert_not_called()

    def test_result_receipt_failure_does_not_allow_reexecution(self):
        preview = self.prepare()
        real = command_run.write_command_receipt
        def interrupted(path, content):
            if path.name.endswith(".finished.json"):
                raise OSError("injected write failure")
            real(path, content)
        with patch.object(command_run, "write_command_receipt", side_effect=interrupted):
            with self.assertRaises(WorkError) as error:
                self.run_cmd(preview["approved_sha256"])
        self.assertEqual(error.exception.code, "command_run_interrupted")
        with patch.object(command_run, "_execute") as execute:
            with self.assertRaises(WorkError):
                self.run_cmd(preview["approved_sha256"])
            execute.assert_not_called()

    def test_output_tails_are_bounded(self):
        self.configure([sys.executable, "-c", "import sys; print('x'*10000); print('error', file=sys.stderr)"])
        result = self.run_cmd(self.prepare()["approved_sha256"])
        self.assertTrue(result["stdout_truncated"])
        self.assertLessEqual(len(result["stdout_tail"]), 4096)
        self.assertIn("error", result["stderr_tail"])

    def test_wrong_lock_and_instruction_drift_are_rejected(self):
        self.index["lock"]["record_id"] = "VAL-001"
        self.save()
        with self.assertRaises(WorkError):
            self.prepare()
        self.index["lock"]["record_id"] = "CMD-001"
        self.index["lock"]["execute_instructions_sha256"] = "0" * 64
        self.attempt["execute_instructions_sha256"] = "0" * 64
        self.save()
        with self.assertRaises(WorkError) as error:
            self.prepare()
        self.assertEqual(error.exception.code, "command_run_execute_instructions_changed")

    def test_cli_requires_approval_and_preview_does_not_create_mutex(self):
        args = self.input_arguments(["--project-root", str(self.root), "execute", "command-prepare",
            "--task-path", self.fixture.artifacts["task"], "--execution-dir", self.fixture.artifacts["execution"],
            "--task-id", "TASK-001", "--user-config-root", str(self.root), "--input-file", "request.json"],
            json.dumps(self.request))
        before = self.snapshot()
        out = io.StringIO()
        self.assertEqual(main(args, stdout=out), 0, out.getvalue())
        self.assertEqual(before, self.snapshot())
        args[3] = "command-run"
        with patch.object(command_run, "_execute") as execute:
            self.assertNotEqual(main(args, stdout=io.StringIO()), 0)
            execute.assert_not_called()
