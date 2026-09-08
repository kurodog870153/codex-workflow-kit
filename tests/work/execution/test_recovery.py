from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
sys.path.insert(0, str(SKILL_ROOT / "scripts"))

from worklib.contracts.attempt import render_attempt_contract, validate_attempt_file
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from worklib.execution.attempt_close import close_attempt
from worklib.execution.record_finish import finish_record
from worklib.execution.recovery import recover_execution
from worklib.foundation.errors import ExitCode, WorkError
from worklib.foundation.markdown import (
    parse_json_contract,
    render_json_contract,
)
from worklib.instructions.selection import build_instruction_selection


class ExecutionTransactionRecoveryTests(unittest.TestCase):
    def exercise_recovery(self, transaction: str, failed_replace: int) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            execution = project / "execution"
            attempt_path = execution / "TASK-001" / "ATTEMPT-001.json"
            attempt_path.parent.mkdir(parents=True)
            index_path = execution / "index.json"
            task_selection = build_instruction_selection(
                skill_root=SKILL_ROOT,
                mode="task",
                selected_paths=["web/backend/java/jpa"],
                reference_names=["task.general.task-records"],
            )
            execute_selection = build_instruction_selection(
                skill_root=SKILL_ROOT,
                mode="execute",
                selected_paths=["web/backend/java/jpa"],
                reference_names=["execute.general.execution-records"],
            )
            task = {
                "id": "TASK-001",
                "skill_id": None,
                "instruction_selection": task_selection,
                "validations": [{"id": "VAL-001"}],
            }
            contract = {
                "requirement_id": "example",
                "spec_id": "TASK-SPEC-001",
                "artifacts": {"task": "task.json", "execution": "execution"},
                "tasks": [task],
            }
            validation = {
                "task_sha256": "a" * 64,
                "instructions_sha256": "b" * 64,
                "task_instructions_sha256": {
                    "TASK-001": task_selection["instructions_sha256"]
                },
                "hierarchy_selection_sha256": "f" * 64,
                "skill_selection_sha256": "d" * 64,
                "task_skill_ids": {"TASK-001": None},
            }
            record = {
                "id": "VAL-001",
                "kind": "validation",
                "outcome": "passed",
                "evidence": "The validation passed.",
            }
            attempt = {
                "schema": "work-attempt/v1",
                "attempt_id": "ATTEMPT-001",
                "task_spec_id": "TASK-SPEC-001",
                "task_id": "TASK-001",
                "skill_id": None,
                "status": "in_progress",
                "task_sha256": validation["task_sha256"],
                "task_instructions_sha256": task_selection["instructions_sha256"],
                "execute_instructions_sha256": execute_selection["instructions_sha256"],
                "hierarchy_selection_sha256": "f" * 64,
                "execute_skill_selection_sha256": "d" * 64,
                "started_at": "2026-09-01T10:00+08:00",
                "records": [record] if transaction == "attempt_close" else [],
            }
            index = build_initial_execution_index(contract, validation)
            index["overall_status"] = "in_progress"
            index["tasks"][0].update(
                {"status": "in_progress", "latest_attempt": "ATTEMPT-001"}
            )
            index["lock"] = {
                "kind": "execution",
                "task_id": "TASK-001",
                "attempt_id": "ATTEMPT-001",
                "execute_instructions_sha256": execute_selection["instructions_sha256"],
            }
            if transaction == "record_finish":
                index["lock"]["record_id"] = "VAL-001"
                request = {
                    "schema": "work-record-finish-request/v1",
                    "record": record,
                }
                operation = finish_record
            else:
                request = {
                    "schema": "work-attempt-close-request/v1",
                    "status": "completed",
                }
                operation = close_attempt

            (project / "task.json").write_bytes(
                render_json_contract(contract)
            )
            original_attempt = render_attempt_contract(attempt, project_root=project)
            original_index = render_execution_index(index)
            attempt_path.write_bytes(original_attempt)
            index_path.write_bytes(original_index)
            common = {
                "source": "test",
                "project_root": project,
                "user_config_root": temporary,
                "raw_task_path": "task.json",
                "raw_execution_dir": "execution",
                "task_id": "TASK-001",
            }
            real_replace = os.replace
            replace_count = 0

            def interrupted_replace(source, target):
                nonlocal replace_count
                replace_count += 1
                if replace_count == failed_replace:
                    raise OSError("simulated replacement failure")
                return real_replace(source, target)

            # Isolate upstream Plan/TASK validation; artifact validation and all
            # transaction writes, identity checks, and recovery remain real.
            with patch(
                f"worklib.execution.{transaction}.validate_task_contract",
                return_value=validation,
            ), patch("os.replace", side_effect=interrupted_replace):
                with self.assertRaises(WorkError) as context:
                    operation(json.dumps(request).encode("utf-8"), **common)
            error = context.exception
            self.assertEqual(error.exit_code, ExitCode.IO_FAILURE)
            self.assertEqual(error.code, f"{transaction}_replace_failed")
            self.assertTrue(error.details["recovery_required"])
            expected_stage = (
                "attempt_update_prepared"
                if failed_replace == 1
                else (
                    "lock_update_prepared"
                    if transaction == "record_finish"
                    else "index_update_prepared"
                )
            )
            self.assertEqual(error.details["transaction_stage"], expected_stage)
            self.assertEqual(index_path.read_bytes(), original_index)
            if failed_replace == 1:
                self.assertEqual(attempt_path.read_bytes(), original_attempt)
            else:
                self.assertNotEqual(attempt_path.read_bytes(), original_attempt)
            transaction_files = sorted(path.name for path in execution.glob(".work-*.tmp"))
            self.assertEqual(len(transaction_files), 1)
            recovery_request = {
                "schema": "work-execution-recovery-request/v1",
                "transaction": transaction,
                "attempt_id": "ATTEMPT-001",
                "transaction_files": transaction_files,
            }
            with patch(
                "worklib.execution.recovery.validate_task_contract",
                return_value=validation,
            ):
                result = recover_execution(
                    json.dumps(recovery_request).encode("utf-8"), **common
                )
            self.assertEqual(result["status"], "recovered")
            self.assertEqual(list(execution.glob(".work-*.tmp")), [])
            validate_attempt_file(project, "execution/TASK-001/ATTEMPT-001.json")
            validate_execution_index(index_path.read_bytes(), source=str(index_path))
            recovered_attempt = parse_json_contract(
                attempt_path.read_bytes(), source=str(attempt_path)
            )
            recovered_index = parse_json_contract(
                index_path.read_bytes(), source=str(index_path)
            )
            self.assertEqual(recovered_attempt["records"], [record])
            expected_index = copy.deepcopy(index)
            if transaction == "record_finish":
                self.assertEqual(recovered_attempt["status"], "in_progress")
                expected_index["lock"].pop("record_id")
                self.assertEqual(result["lock_status"], "attempt_held")
            else:
                self.assertEqual(recovered_attempt["status"], "completed")
                expected_index.pop("lock")
                expected_index["overall_status"] = "completed"
                expected_index["tasks"][0]["status"] = "completed"
                self.assertEqual(result["lock_status"], "released")
            self.assertEqual(recovered_index, expected_index)

    def test_record_finish_recovers_after_each_replacement_failure(self) -> None:
        for failed_replace in (1, 2):
            with self.subTest(failed_replace=failed_replace):
                self.exercise_recovery("record_finish", failed_replace)

    def test_attempt_close_recovers_after_each_replacement_failure(self) -> None:
        for failed_replace in (1, 2):
            with self.subTest(failed_replace=failed_replace):
                self.exercise_recovery("attempt_close", failed_replace)


if __name__ == "__main__":
    unittest.main()
