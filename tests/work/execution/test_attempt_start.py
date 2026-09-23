from __future__ import annotations

import sys
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.common.errors import ExitCode, WorkError
from worklib.business_services.execution.attempt_start import (
    _build_attempt,
    _lock,
    _raise_transaction_error,
    _transaction_stage,
    _validate_snapshot,
    prepare_attempt_start,
)
from worklib.services.attempt.lock import build_execution_lock
from worklib.services.attempt.validation import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from worklib.services.attempt import minimal_authorization


class ExecuteInstructionAttemptStartTests(unittest.TestCase):
    def test_prepare_derives_exact_authorization_and_retry_source(self) -> None:
        task = {"id": "TASK-001", "commands": [{"id": "CMD-001", "mode": "argv", "argv": ["tool"]}],
                "validations": [], "operations": [],
                "files": [{"id": "FILE-001", "action": "modify", "path": "src.txt"}]}
        operations = SimpleNamespace(load_task_execution_context=lambda *args, **kwargs: {
            "contract": {"tasks": [task], "execution_defaults": {"working_directory": ".", "os": "windows", "shell": "powershell"}}})
        choice = {"command_positions": [1], "validation_positions": [], "modifiable_files": ["src.txt"],
                  "external_operation_positions": [], "allowed_deviations": [],
                  "authorization_evidence": "User approved the exact scope.", "carried_records": []}
        worktree = {"task_status": "pending", "snapshot_sha256": "a" * 64}
        index = {"tasks": [{"id": "TASK-001", "status": "pending"}]}
        with patch("worklib.business_services.execution.attempt_start.inspect_execute_worktree", return_value=worktree), \
             patch("worklib.business_services.execution.attempt_start._read_index", return_value=(b"", index)):
            result = prepare_attempt_start(json.dumps(choice).encode(), source="test",
                project_root=REPO_ROOT, user_config_root=str(REPO_ROOT), raw_task_path="task.json",
                raw_execution_dir="outputs/work/executions/example", task_id="TASK-001", operations=operations)
        request = result["request"]
        self.assertEqual(result["schema"], "work-attempt-start-prepare/v1")
        self.assertEqual(request["authorization"]["commands"], task["commands"])
        self.assertEqual(request["authorization"]["working_directories"], ["."])
        self.assertEqual(request["worktree_snapshot_sha256"], "a" * 64)
        self.assertNotIn("continuation", request)
        index["tasks"][0].update(status="pending_retry", latest_attempt="ATTEMPT-001")
        worktree["task_status"] = "pending_retry"
        source = {"records": [{"id": "CMD-001"}], "carried_records": []}
        with patch("worklib.business_services.execution.attempt_start.inspect_execute_worktree", return_value=worktree), \
             patch("worklib.business_services.execution.attempt_start._read_index", return_value=(b"", index)), \
             patch("worklib.business_services.execution.attempt_start._load_source_attempt", return_value=source):
            retry = prepare_attempt_start(json.dumps(choice).encode(), source="test",
                project_root=REPO_ROOT, user_config_root=str(REPO_ROOT), raw_task_path="task.json",
                raw_execution_dir="outputs/work/executions/example", task_id="TASK-001", operations=operations)
        self.assertEqual(retry["request"]["continuation"]["source_attempt_id"], "ATTEMPT-001")
        choice.pop("carried_records")
        with patch("worklib.business_services.execution.attempt_start.inspect_execute_worktree", return_value=worktree), \
             patch("worklib.business_services.execution.attempt_start._read_index", return_value=(b"", index)), \
             patch("worklib.business_services.execution.attempt_start._load_source_attempt", return_value=source):
            omitted = prepare_attempt_start(json.dumps(choice).encode(), source="test",
                project_root=REPO_ROOT, user_config_root=str(REPO_ROOT), raw_task_path="task.json",
                raw_execution_dir="outputs/work/executions/example", task_id="TASK-001", operations=operations)
        self.assertEqual(omitted["request"]["continuation"]["carried_records"], [])

    def test_prepare_rejects_formal_scope_and_deviation_ids(self) -> None:
        from worklib.models.execution.attempt_start import AttemptStartPrepareRequestContract
        base = dict(AttemptStartPrepareRequestContract.contract_example)
        for addition in ({"command_ids": ["CMD-001"]},
                         {"allowed_deviations": [{"anchor_kind": "command", "anchor_position": 1,
                           "action": {"kind": "skip_record", "record_id": "CMD-001", "reason": "skip"}}]},
                         {"carried_records": [{"record_id": "CMD-001", "evidence": "valid"}]}):
            with self.subTest(addition=addition), self.assertRaises(WorkError):
                AttemptStartPrepareRequestContract.parse_json_bytes(json.dumps({**base, **addition}).encode(), source="test")

    def test_prepare_formalizes_deviation_and_carried_position(self) -> None:
        task = {"id": "TASK-001", "commands": [{"id": "CMD-001", "mode": "argv", "argv": ["tool"]}],
                "validations": [], "operations": [], "files": []}
        operations = SimpleNamespace(load_task_execution_context=lambda *args, **kwargs: {
            "contract": {"tasks": [task], "execution_defaults": {"working_directory": ".", "os": "windows", "shell": "powershell"}}})
        choice = {"command_positions": [1], "validation_positions": [], "modifiable_files": [],
                  "external_operation_positions": [], "allowed_deviations": [{"anchor_kind": "command", "anchor_position": 1,
                      "action": {"kind": "replace_command", "replacement": {"mode": "argv", "argv": ["tool", "fixed"]}}}],
                  "authorization_evidence": "User approved this scope.", "carried_records": [{"position": 1, "evidence": "Still valid."}]}
        worktree = {"task_status": "pending_retry", "snapshot_sha256": "a" * 64}
        index = {"tasks": [{"id": "TASK-001", "status": "pending_retry", "latest_attempt": "ATTEMPT-001"}]}
        source = {"records": [{"id": "CMD-001"}], "carried_records": []}
        with patch("worklib.business_services.execution.attempt_start.inspect_execute_worktree", return_value=worktree), \
             patch("worklib.business_services.execution.attempt_start._read_index", return_value=(b"", index)), \
             patch("worklib.business_services.execution.attempt_start._load_source_attempt", return_value=source):
            result = prepare_attempt_start(json.dumps(choice).encode(), source="test", project_root=REPO_ROOT,
                user_config_root=str(REPO_ROOT), raw_task_path="task.json", raw_execution_dir="outputs/work/executions/example",
                task_id="TASK-001", operations=operations)
        self.assertEqual(result["request"]["authorization"]["allowed_deviations"][0]["record_id"], "CMD-001")
        self.assertEqual(result["request"]["continuation"]["carried_records"][0]["record_id"], "CMD-001")

    def test_prepare_rejects_old_complete_authorization(self) -> None:
        with self.assertRaises(WorkError) as caught:
            prepare_attempt_start(json.dumps({"authorization": minimal_authorization()}).encode(),
                source="test", project_root=REPO_ROOT, user_config_root=str(REPO_ROOT),
                raw_task_path="task.json", raw_execution_dir="outputs/work/executions/example",
                task_id="TASK-001")
        self.assertEqual(caught.exception.code, "invalid_object_fields")

    def test_transaction_stage_and_error_recovery_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            index_path = directory / "index.json"
            attempt_path = directory / "attempt.json"
            lock_temporary = directory / "lock.tmp"
            started_temporary = directory / "started.tmp"
            index_path.write_bytes(render_execution_index(self.index()))

            self.assertEqual(
                _transaction_stage(
                    index_path=index_path,
                    attempt_path=attempt_path,
                    lock_temporary=lock_temporary,
                    started_temporary=started_temporary,
                    attempt_id="ATTEMPT-001",
                ),
                "not_started",
            )
            attempt_path.write_bytes(b"attempt")
            original = WorkError(
                ExitCode.IO_FAILURE, "write_failed", "Write failed."
            )
            with self.assertRaises(WorkError) as context:
                _raise_transaction_error(
                    original,
                    index_path=index_path,
                    attempt_path=attempt_path,
                    lock_temporary=lock_temporary,
                    started_temporary=started_temporary,
                    attempt_id="ATTEMPT-001",
                )

            self.assertTrue(context.exception.details["recovery_required"])
            self.assertEqual(
                context.exception.details["transaction_stage"],
                "attempt_created",
            )

    def test_legacy_lock_symbol_uses_attempt_lock_service(self) -> None:
        self.assertIs(_lock, build_execution_lock)

    @patch("worklib.business_services.execution.attempt_start.worktree_snapshot_sha256", return_value="actual")
    @patch("worklib.business_services.execution.attempt_start.collect_git_status", return_value=[])
    def test_snapshot_mismatch_reports_expected_and_actual(
        self, _mocked_status, _mocked_snapshot
    ) -> None:
        with self.assertRaises(WorkError) as context:
            _validate_snapshot(
                project_root=REPO_ROOT,
                execution_dir="outputs/work/executions/example",
                expected="expected",
            )

        self.assertEqual(context.exception.code, "attempt_start_worktree_snapshot_changed")
        self.assertEqual(context.exception.details["expected"], "expected")
        self.assertEqual(context.exception.details["actual"], "actual")

    def preflight(self) -> dict[str, object]:
        return {
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "task_collection_sha256": "a" * 64,
            "task_index_sha256": "1" * 64,
            "task_item_sha256": "2" * 64,
            "task_instructions_sha256": "b" * 64,
            "execute_instructions_sha256": "c" * 64,
            "hierarchy_selection_sha256": "f" * 64,
            "execute_skill_selection": {"selection_sha256": "e" * 64},
            "execution_dir": "outputs/work/executions/example",
        }

    def index(self) -> dict[str, object]:
        return build_initial_execution_index(
            {
                "requirement_id": "example",
                "spec_id": "TASK-SPEC-001",
                "tasks": [{"id": "TASK-001", "skill_id": None}],
            },
            {
                "schema": "work-task-collection-validation/v1",
                "task_collection_sha256": "a" * 64,
                "task_index_sha256": "1" * 64,
                "task_item_sha256": {"TASK-001": "2" * 64},
                "instructions_sha256": "d" * 64,
                "task_instructions_sha256": {"TASK-001": "b" * 64},
                "hierarchy_selection_sha256": "f" * 64,
                "skill_selection_sha256": "e" * 64,
                "task_skill_ids": {"TASK-001": None},
            },
        )

    def attempt(self) -> dict[str, object]:
        return _build_attempt(
            project_root=REPO_ROOT,
            preflight=self.preflight(),
            index=self.index(),
            request={"authorization": minimal_authorization()},
            original_status="pending",
            attempt_id="ATTEMPT-001",
            started_at="2026-09-01T10:00+08:00",
        )

    def test_builds_attempt_with_instruction_fingerprints(self) -> None:
        attempt = self.attempt()

        self.assertEqual(attempt["schema"], "work-attempt/v1")
        self.assertEqual(attempt["task_collection_sha256"], "a" * 64)
        self.assertEqual(attempt["task_index_sha256"], "1" * 64)
        self.assertEqual(attempt["task_item_sha256"], "2" * 64)
        self.assertEqual(attempt["task_instructions_sha256"], "b" * 64)
        self.assertEqual(attempt["execute_instructions_sha256"], "c" * 64)
        self.assertEqual(attempt["hierarchy_selection_sha256"], "f" * 64)
        self.assertIsNone(attempt["skill_id"])
        self.assertEqual(attempt["execute_skill_selection_sha256"], "e" * 64)
        self.assertNotIn("task_rules_sha256", attempt)
        self.assertNotIn("execute_rules_sha256", attempt)
        self.assertNotIn("task_sha256", attempt)

    @patch("worklib.business_services.execution.attempt_start._load_source_attempt")
    def test_continuation_rejects_changed_skill_identity(self, mocked_load) -> None:
        index = self.index()
        row = index["tasks"][0]  # type: ignore[index]
        row["status"] = "pending_retry"
        row["latest_attempt"] = "ATTEMPT-001"
        row["status_reason"] = {"kind": "attempt", "ref": "ATTEMPT-001"}
        mocked_load.return_value = {
            "skill_id": "different",
            "hierarchy_selection_sha256": "f" * 64,
            "execute_skill_selection_sha256": "e" * 64,
            "records": [],
        }

        with self.assertRaises(WorkError) as context:
            _build_attempt(
                project_root=REPO_ROOT,
                preflight=self.preflight(),
                index=index,
                request={
                    "authorization": minimal_authorization(),
                    "continuation": {
                        "source_attempt_id": "ATTEMPT-001",
                        "carried_records": [],
                    }
                },
                original_status="pending_retry",
                attempt_id="ATTEMPT-002",
                started_at="2026-09-01T10:05+08:00",
            )

        self.assertEqual(
            context.exception.code,
            "attempt_start_continuation_skill_identity_mismatch",
        )

    def test_execution_lock_uses_instruction_fingerprint(self) -> None:
        index = self.index()
        index["lock"] = _lock(
            task_id="TASK-001",
            attempt_id="ATTEMPT-001",
            execute_instructions_sha256="c" * 64,
        )

        result = validate_execution_index(
            render_execution_index(index),
            source="test",
            expected=index,
        )

        self.assertEqual(result["overall_status"], "pending")
        self.assertEqual(
            index["lock"]["execute_instructions_sha256"],  # type: ignore[index]
            "c" * 64,
        )
        self.assertNotIn("execute_rules_sha256", index["lock"])  # type: ignore[operator]


if __name__ == "__main__":
    unittest.main()
