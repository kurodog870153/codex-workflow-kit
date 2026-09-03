from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
SCRIPT_ROOT = SKILL_ROOT / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.execution.record_begin import (
    _validate_execute_instructions,
    _validate_identity,
)
from worklib.contracts.execution_index import build_initial_execution_index
from worklib.instructions.selection import build_instruction_selection


class ExecuteInstructionRecordTests(unittest.TestCase):
    def setUp(self) -> None:
        self.task_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="task",
            selected_paths=["web/backend/java/jpa"],
            reference_names=["task.general.task-records"],
        )
        self.execute_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="execute",
            selected_paths=["web/backend/java/jpa"],
            reference_names=["execute.general.execution-records"],
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
            "execute_instructions_sha256": self.execute_selection[
                "instructions_sha256"
            ],
        }

    def test_record_identity_accepts_instruction_fingerprints(self) -> None:
        row = _validate_identity(
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
            _validate_identity(
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

    def test_begin_and_finish_validate_execute_instruction_selection(self) -> None:
        task = self.task_contract["tasks"][0]

        for operation in ("record_begin", "record_finish"):
            with self.subTest(operation=operation):
                current = _validate_execute_instructions(
                    task,
                    self.attempt,
                    operation=operation,
                )
                self.assertEqual(current, self.execute_selection)

    def test_retry_loads_recovery_instruction_reference(self) -> None:
        task = self.task_contract["tasks"][0]
        attempt = copy.deepcopy(self.attempt)
        attempt["continued_from"] = "ATTEMPT-001"
        recovery = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="execute",
            selected_paths=["web/backend/java/jpa"],
            reference_names=[
                "execute.general.execution-records",
                "execute.general.execution-recovery",
            ],
        )
        attempt["execute_instructions_sha256"] = recovery["instructions_sha256"]

        current = _validate_execute_instructions(
            task,
            attempt,
            operation="record_begin",
        )

        self.assertEqual(
            current["references"],
            [
                "execute.general.execution-records",
                "execute.general.execution-recovery",
            ],
        )

    def test_rejects_stale_execute_instruction_fingerprint(self) -> None:
        task = self.task_contract["tasks"][0]
        attempt = copy.deepcopy(self.attempt)
        attempt["execute_instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            _validate_execute_instructions(
                task,
                attempt,
                operation="record_finish",
            )

        self.assertEqual(
            context.exception.code,
            "record_finish_execute_instructions_changed",
        )


if __name__ == "__main__":
    unittest.main()
