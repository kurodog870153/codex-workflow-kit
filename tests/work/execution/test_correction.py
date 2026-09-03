from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
SCRIPT_ROOT = SKILL_ROOT / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.correction import canonicalize_correction_contract
from worklib.foundation.errors import WorkError
from worklib.execution.correction import _build_lock
from worklib.execution.record_begin import _validate_execute_instructions
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from worklib.instructions.selection import build_instruction_selection


class ExecuteInstructionCorrectionTests(unittest.TestCase):
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
        self.task = {
            "id": "TASK-001",
            "instruction_selection": self.task_selection,
        }
        self.attempt = {
            "task_instructions_sha256": self.task_selection[
                "instructions_sha256"
            ],
            "execute_instructions_sha256": self.execute_selection[
                "instructions_sha256"
            ],
        }
        self.correction = {
            "schema": "work-correction/v1",
            "correction_id": "ATTEMPT-001-CORRECTION-001",
            "created_at": "2026-09-01T10:05+08:00",
            "target_attempt_id": "ATTEMPT-001",
            "task_instructions_sha256": self.attempt[
                "task_instructions_sha256"
            ],
            "execute_instructions_sha256": self.attempt[
                "execute_instructions_sha256"
            ],
            "field": "records[0].outcome",
            "correct_value": "passed",
            "reason": "Correct the recorded outcome.",
        }

    def test_correction_contract_uses_instruction_fingerprints(self) -> None:
        canonical = canonicalize_correction_contract(self.correction)

        self.assertEqual(
            canonical["task_instructions_sha256"],
            self.task_selection["instructions_sha256"],
        )
        self.assertNotIn("task_rules_sha256", canonical)
        self.assertNotIn("execute_rules_sha256", canonical)

    def test_correction_contract_rejects_legacy_rule_fingerprints(self) -> None:
        correction = copy.deepcopy(self.correction)
        correction["task_rules_sha256"] = correction.pop(
            "task_instructions_sha256"
        )
        correction["execute_rules_sha256"] = correction.pop(
            "execute_instructions_sha256"
        )

        with self.assertRaises(WorkError) as context:
            canonicalize_correction_contract(correction)

        self.assertEqual(context.exception.code, "correction_invalid_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["execute_instructions_sha256", "task_instructions_sha256"],
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["execute_rules_sha256", "task_rules_sha256"],
        )

    def test_correction_lock_uses_execute_instruction_fingerprint(self) -> None:
        index = build_initial_execution_index(
            {
                "requirement_id": "example",
                "spec_id": "TASK-SPEC-001",
                "tasks": [{"id": "TASK-001", "skill_id": None}],
            },
            {
                "task_sha256": "a" * 64,
                "instructions_sha256": "b" * 64,
                "task_instructions_sha256": {"TASK-001": "c" * 64},
                "hierarchy_selection_sha256": "f" * 64,
                "skill_selection_sha256": "d" * 64,
                "task_skill_ids": {"TASK-001": None},
            },
        )
        index["lock"] = _build_lock(
            task_id="TASK-001",
            attempt_id="ATTEMPT-001",
            correction_id="ATTEMPT-001-CORRECTION-001",
            execute_instructions_sha256=self.attempt[
                "execute_instructions_sha256"
            ],
            invalidates_completion=True,
            affected_task_ids=["TASK-001"],
        )

        validate_execution_index(
            render_execution_index(index),
            source="test",
            expected=index,
        )

        self.assertIn("execute_instructions_sha256", index["lock"])
        self.assertNotIn("execute_rules_sha256", index["lock"])

    def test_command_correction_validates_execute_instructions(self) -> None:
        current = _validate_execute_instructions(
            self.task,
            self.attempt,
            operation="command_correction",
        )

        self.assertEqual(current, self.execute_selection)

    def test_command_correction_rejects_stale_execute_instructions(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt["execute_instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            _validate_execute_instructions(
                self.task,
                attempt,
                operation="command_correction",
            )

        self.assertEqual(
            context.exception.code,
            "command_correction_execute_instructions_changed",
        )


if __name__ == "__main__":
    unittest.main()
