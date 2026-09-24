from __future__ import annotations

import copy
import json
import os
import sys
import unittest
from contextlib import nullcontext
from pathlib import Path
from unittest.mock import patch


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
sys.path.insert(0, str(SKILL_ROOT / "scripts"))

from worklib.services.attempt import render_attempt_contract, validate_attempt_file
from worklib.services.attempt import authorization_sha256, minimal_authorization
from tests.work.contracts import test_task_collection
from worklib.services.attempt.validation import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from worklib.models.execution import ExecutionDeviationProposalContract
from worklib.orchestration.execution import close_attempt
from worklib.orchestration.execution import finish_record, record_execution_deviation
from worklib.orchestration.execution import recover_execution
from worklib.models.common.errors import ExitCode, WorkError
from worklib.technical.infrastructure.json_contract import (
    parse_json_contract,
)
from worklib.business_services.task import load_task_collection
from worklib.business_services.instruction import build_instruction_selection


class ExecutionTransactionRecoveryTests(unittest.TestCase):
    def exercise_recovery(self, transaction: str, failed_replace: int) -> None:
        fixture = test_task_collection.TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        project = fixture.root
        validation = load_task_collection(project, str(project), fixture.index_path)
        contract = validation["collection_contract"]
        execution_relative = contract["artifacts"]["execution"]
        execution = project / execution_relative
        attempt_path = execution / "TASK-001" / "ATTEMPT-001" / "attempt.json"
        attempt_path.parent.mkdir(parents=True)
        index_path = execution / "index.json"
        task_selection = contract["tasks"][0]["instruction_selection"]
        execute_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="execute",
            selected_paths=task_selection["selected_paths"],
            reference_names=["execute.general.execution-records"],
        )
        record = {
            "id": "VAL-001",
            "kind": "validation",
            "outcome": "passed",
            "evidence": "The validation passed.",
        }
        attempt = {
            "schema": "work-attempt/v1",
            "attempt_id": "ATTEMPT-001",
            "task_spec_id": contract["spec_id"],
            "task_id": "TASK-001",
            "skill_id": None,
            "status": "in_progress",
            "task_collection_sha256": validation["task_collection_sha256"],
            "task_index_sha256": validation["task_index_sha256"],
            "task_item_sha256": validation["task_item_sha256"]["TASK-001"],
            "task_instructions_sha256": task_selection["instructions_sha256"],
            "execute_instructions_sha256": execute_selection["instructions_sha256"],
            "hierarchy_selection_sha256": validation["hierarchy_selection_sha256"],
            "execute_skill_selection_sha256": validation["skill_selection_sha256"],
            "authorization": minimal_authorization(),
            "authorization_sha256": authorization_sha256(minimal_authorization()),
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
        operation_options = {}
        preparation = nullcontext()
        if transaction == "record_finish":
            index["lock"]["record_id"] = "VAL-001"
            request = {
                "schema": "work-record-finish-request/v1",
                "record": {"outcome": record["outcome"], "evidence": record["evidence"]},
            }
            operation = finish_record
        elif transaction == "deviation_record":
            index["lock"]["record_id"] = "CMD-001"
            request = copy.deepcopy(ExecutionDeviationProposalContract.contract_example)
            approved_sha256 = "9" * 64
            operation_options = {
                "approved_sha256": approved_sha256,
                "authorization_evidence": "User approved this exact runtime deviation.",
            }
            preparation = patch(
                "worklib.business_services.execution.deviation._prepare_execution_deviation",
                return_value={
                    "proposal": request,
                    "preview_sha256": approved_sha256,
                },
            )
            operation = record_execution_deviation
        else:
            request = {
                "schema": "work-attempt-close-request/v1",
                "status": "completed",
            }
            operation = close_attempt

        original_attempt = render_attempt_contract(attempt, project_root=project)
        original_index = render_execution_index(index)
        attempt_path.write_bytes(original_attempt)
        index_path.write_bytes(original_index)
        common = {
            "source": "test",
            "project_root": project,
            "user_config_root": str(project),
            "raw_task_path": fixture.index_path,
            "raw_execution_dir": execution_relative,
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

        with preparation, patch("os.replace", side_effect=interrupted_replace):
            with self.assertRaises(WorkError) as context:
                operation(
                    json.dumps(request).encode("utf-8"),
                    **common,
                    **operation_options,
                )
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
        result = recover_execution(
            json.dumps(recovery_request).encode("utf-8"), **common
        )
        self.assertEqual(result["status"], "recovered")
        self.assertEqual(list(execution.glob(".work-*.tmp")), [])
        validate_attempt_file(
            project,
            f"{execution_relative}/TASK-001/ATTEMPT-001/attempt.json",
        )
        validate_execution_index(index_path.read_bytes(), source=str(index_path))
        recovered_attempt = parse_json_contract(
            attempt_path.read_bytes(), source=str(attempt_path)
        )
        recovered_index = parse_json_contract(
            index_path.read_bytes(), source=str(index_path)
        )
        self.assertEqual(
            recovered_attempt["records"],
            [] if transaction == "deviation_record" else [record],
        )
        expected_index = copy.deepcopy(index)
        if transaction == "record_finish":
            self.assertEqual(recovered_attempt["status"], "in_progress")
            expected_index["lock"].pop("record_id")
            self.assertEqual(result["lock_status"], "attempt_held")
        elif transaction == "deviation_record":
            self.assertEqual(recovered_attempt["status"], "in_progress")
            self.assertEqual(
                recovered_attempt["execution_deviations"][0]["approved_preview_sha256"],
                "9" * 64,
            )
            self.assertEqual(result["record_id"], "CMD-001")
            self.assertEqual(result["lock_status"], "record_reserved")
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

    def test_deviation_record_recovers_after_replacement_failure(self) -> None:
        self.exercise_recovery("deviation_record", 1)


if __name__ == "__main__":
    unittest.main()
