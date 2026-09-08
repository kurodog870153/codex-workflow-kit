from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
SCRIPT_ROOT = SKILL_ROOT / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import ExitCode, WorkError
from worklib.execution.context import (
    find_task_row,
    read_contract,
    validate_execution_identity,
)
from worklib.contracts.execution_index import build_initial_execution_index
from worklib.instructions.selection import build_instruction_selection


class ExecutionContextTests(unittest.TestCase):
    def setUp(self) -> None:
        self.task_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="task",
            selected_paths=["web/backend/java/jpa"],
            reference_names=["task.general.task-records"],
        )
        self.task_contract = {
            "requirement_id": "example",
            "spec_id": "TASK-SPEC-001",
            "tasks": [
                {
                    "id": "TASK-001",
                    "skill_id": None,
                    "instruction_selection": self.task_selection,
                }
            ],
        }
        self.task_validation = {
            "task_sha256": "a" * 64,
            "instructions_sha256": "b" * 64,
            "task_instructions_sha256": {
                "TASK-001": self.task_selection["instructions_sha256"]
            },
            "skill_selection_sha256": "d" * 64,
            "hierarchy_selection_sha256": "f" * 64,
            "task_skill_ids": {"TASK-001": None},
        }
        self.index = build_initial_execution_index(
            self.task_contract,
            self.task_validation,
        )
        self.attempt = {
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "task_sha256": "a" * 64,
            "task_instructions_sha256": self.task_selection[
                "instructions_sha256"
            ],
            "hierarchy_selection_sha256": "f" * 64,
        }

    def test_record_identity_accepts_instruction_fingerprints(self) -> None:
        row = validate_execution_identity(
            task_contract=self.task_contract,
            task_validation=self.task_validation,
            index=self.index,
            attempt=self.attempt,
            task_id="TASK-001",
        )

        self.assertEqual(
            row["instructions_sha256"],
            self.task_selection["instructions_sha256"],
        )

    def test_record_identity_rejects_stale_task_instruction_fingerprint(self) -> None:
        index = copy.deepcopy(self.index)
        index["tasks"][0]["instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            validate_execution_identity(
                task_contract=self.task_contract,
                task_validation=self.task_validation,
                index=index,
                attempt=self.attempt,
                task_id="TASK-001",
            )

        self.assertEqual(
            context.exception.code,
            "record_begin_task_instructions_mismatch",
        )

    def test_read_contract_preserves_original_bytes(self) -> None:
        raw = b'{"value": 1}\r\n'
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "contract.json"
            path.write_bytes(raw)
            self.assertEqual(read_contract(path), (raw, {"value": 1}))

    def test_missing_task_preserves_error_contract(self) -> None:
        with self.assertRaises(WorkError) as context:
            find_task_row(self.index, "TASK-002")
        self.assertEqual(context.exception.exit_code, ExitCode.WORKFLOW_STATE)
        self.assertEqual(context.exception.code, "record_begin_task_not_found")
        self.assertEqual(context.exception.details, {"task_id": "TASK-002"})

    def test_identity_rejects_index_task_set_and_attempt_drift(self) -> None:
        for mismatch, expected_code in (
            ("index", "record_begin_index_identity_mismatch"),
            ("task_set", "record_begin_index_task_set_mismatch"),
            ("attempt", "record_begin_attempt_identity_mismatch"),
        ):
            with self.subTest(mismatch=mismatch):
                index = copy.deepcopy(self.index)
                attempt = copy.deepcopy(self.attempt)
                if mismatch == "index":
                    index["task_sha256"] = "0" * 64
                elif mismatch == "task_set":
                    index["tasks"][0]["id"] = "TASK-002"
                else:
                    attempt["task_sha256"] = "0" * 64
                with self.assertRaises(WorkError) as context:
                    validate_execution_identity(
                        task_contract=self.task_contract,
                        task_validation=self.task_validation,
                        index=index,
                        attempt=attempt,
                        task_id="TASK-001",
                    )
                self.assertEqual(context.exception.exit_code, ExitCode.ARTIFACT_INTEGRITY)
                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
