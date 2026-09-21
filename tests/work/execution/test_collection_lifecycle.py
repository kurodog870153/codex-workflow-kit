from __future__ import annotations

import copy
import json
import os
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
sys.path.insert(0, str(SKILL_ROOT / "scripts"))

from tests.work.contracts import test_task_collection
from worklib.business_services.task import load_task_collection
from worklib.services.attempt.validation import (
    build_initial_execution_index,
    render_execution_index,
)
from worklib.workflows.execution import close_attempt, start_attempt
from worklib.workflows.execution import create_correction
from worklib.workflows.execution import begin_record, finish_record
from worklib.workflows.execution import recover_execution
from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.json_contract import parse_json_contract
from worklib.business_services.instruction import build_instruction_selection
from worklib.services.attempt import minimal_authorization


class CollectionExecutionLifecycleTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture = test_task_collection.TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.project = fixture.root
        self.task_path = fixture.index_path
        self.validation = load_task_collection(
            self.project, str(self.project), self.task_path
        )
        self.contract = self.validation["collection_contract"]
        self.execution_dir = self.contract["artifacts"]["execution"]
        self.execution = self.project / self.execution_dir
        self.execution.mkdir(parents=True, exist_ok=True)
        self.index_path = self.execution / "index.json"
        self.index_path.write_bytes(
            render_execution_index(
                build_initial_execution_index(self.contract, self.validation)
            )
        )
        execute_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="execute",
            selected_paths=[],
            reference_names=["execute.general.execution-records"],
        )
        self.preflight = {
            "task_spec_id": self.contract["spec_id"],
            "task_id": "TASK-001",
            "skill_id": None,
            "task_collection_sha256": self.validation[
                "task_collection_sha256"
            ],
            "task_index_sha256": self.validation["task_index_sha256"],
            "task_item_sha256": self.validation["task_item_sha256"][
                "TASK-001"
            ],
            "task_instructions_sha256": self.validation[
                "task_instructions_sha256"
            ]["TASK-001"],
            "execute_instructions_sha256": execute_selection[
                "instructions_sha256"
            ],
            "hierarchy_selection_sha256": self.validation[
                "hierarchy_selection_sha256"
            ],
            "execute_skill_selection": {
                "selection_sha256": self.validation[
                    "skill_selection_sha256"
                ]
            },
            "execution_dir": self.execution_dir,
            "snapshot_sha256": "e" * 64,
        }
        self.common = {
            "project_root": self.project,
            "user_config_root": str(self.project),
            "raw_task_path": self.task_path,
            "raw_execution_dir": self.execution_dir,
            "task_id": "TASK-001",
        }

    def start(self) -> Path:
        authorization = minimal_authorization()
        authorization["validations"] = [
            copy.deepcopy(self.contract["tasks"][0]["validations"][0])
        ]
        request = {
            "schema": "work-attempt-start-request/v1",
            "worktree_snapshot_sha256": self.preflight["snapshot_sha256"],
            "authorization": authorization,
        }
        with patch(
            "worklib.business_services.execution.attempt_start.inspect_execute_worktree",
            return_value=self.preflight,
        ):
            result = start_attempt(
                json.dumps(request).encode("utf-8"),
                source="test",
                **self.common,
            )
        return self.project / result["attempt_path"]

    def finish_validation(self) -> None:
        reserved = begin_record(base_record_id="VAL-001", **self.common)
        finish_record(
            json.dumps(
                {
                    "schema": "work-record-finish-request/v1",
                    "record": {
                        "id": reserved["record_id"],
                        "kind": "validation",
                        "outcome": "passed",
                        "evidence": "The approved check passed.",
                    },
                }
            ).encode("utf-8"),
            source="test",
            **self.common,
        )

    def assert_collection_fingerprints(self, contract: dict[str, object]) -> None:
        self.assertNotIn("task_sha256", contract)
        for field in (
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
        ):
            self.assertEqual(contract[field], self.preflight[field])

    def test_start_close_and_correction_preserve_collection_fingerprints(
        self,
    ) -> None:
        attempt_path = self.start()
        attempt = parse_json_contract(
            attempt_path.read_bytes(), source=str(attempt_path)
        )
        self.assertEqual(attempt["schema"], "work-attempt/v1")
        self.assert_collection_fingerprints(attempt)

        self.finish_validation()
        close_attempt(
            json.dumps(
                {
                    "schema": "work-attempt-close-request/v1",
                    "status": "completed",
                }
            ).encode("utf-8"),
            source="test",
            **self.common,
        )
        result = create_correction(
            json.dumps(
                {
                    "schema": "work-correction-create-request/v1",
                    "target_attempt_id": "ATTEMPT-001",
                    "field": "records[0].evidence",
                    "correct_value": "The approved manual check passed.",
                    "reason": "Clarify the recorded evidence.",
                    "invalidates_completion": False,
                }
            ).encode("utf-8"),
            source="test",
            **self.common,
        )
        correction_path = self.project / result["correction_path"]
        correction = parse_json_contract(
            correction_path.read_bytes(), source=str(correction_path)
        )
        self.assertEqual(correction["schema"], "work-correction/v1")
        self.assert_collection_fingerprints(correction)

    def test_interrupted_close_recovery_preserves_v1_attempt(self) -> None:
        attempt_path = self.start()
        self.finish_validation()
        real_replace = os.replace
        replace_count = 0

        def interrupted_replace(source, target):
            nonlocal replace_count
            replace_count += 1
            if replace_count == 1:
                raise OSError("simulated replacement failure")
            return real_replace(source, target)

        with patch("os.replace", side_effect=interrupted_replace):
            with self.assertRaises(WorkError):
                close_attempt(
                    json.dumps(
                        {
                            "schema": "work-attempt-close-request/v1",
                            "status": "completed",
                        }
                    ).encode("utf-8"),
                    source="test",
                    **self.common,
                )

        transaction_files = sorted(
            path.name for path in self.execution.glob(".work-*.tmp")
        )
        result = recover_execution(
            json.dumps(
                {
                    "schema": "work-execution-recovery-request/v1",
                    "transaction": "attempt_close",
                    "attempt_id": "ATTEMPT-001",
                    "transaction_files": transaction_files,
                }
            ).encode("utf-8"),
            source="test",
            **self.common,
        )
        self.assertEqual(result["status"], "recovered")
        recovered = parse_json_contract(
            attempt_path.read_bytes(), source=str(attempt_path)
        )
        self.assertEqual(recovered["schema"], "work-attempt/v1")
        self.assertEqual(recovered["status"], "completed")
        self.assert_collection_fingerprints(recovered)


if __name__ == "__main__":
    unittest.main()
