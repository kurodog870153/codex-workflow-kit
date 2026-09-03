from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
SCRIPT_ROOT = SKILL_ROOT / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.attempt import canonicalize_attempt_contract
from worklib.foundation.errors import WorkError
from worklib.execution.attempt_close import (
    _task_status,
    _validate_execute_instruction_close_state,
)
from worklib.instructions.selection import build_instruction_selection


class ExecuteInstructionAttemptCloseTests(unittest.TestCase):
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
            "schema": "work-attempt/v1",
            "attempt_id": "ATTEMPT-001",
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "status": "in_progress",
            "task_sha256": "a" * 64,
            "task_instructions_sha256": self.task_selection[
                "instructions_sha256"
            ],
            "execute_instructions_sha256": self.execute_selection[
                "instructions_sha256"
            ],
            "hierarchy_selection_sha256": "f" * 64,
            "execute_skill_selection_sha256": "d" * 64,
            "started_at": "2026-09-01T10:00+08:00",
            "records": [],
        }

    def stopped_request(self, final_type: str) -> dict[str, str]:
        return {
            "schema": "work-attempt-close-request/v1",
            "status": "stopped",
            "final_type": final_type,
            "reason": "Stop for the recorded reason.",
        }

    def test_attempt_contract_accepts_instructions_changed(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt.update(
            {
                "status": "stopped",
                "final_type": "instructions_changed",
                "reason": "The Execute instructions changed.",
                "ended_at": "2026-09-01T10:05+08:00",
            }
        )

        canonical = canonicalize_attempt_contract(
            attempt,
            project_root=SKILL_ROOT.parent.parent,
        )

        self.assertEqual(canonical["final_type"], "instructions_changed")

    def test_attempt_contract_rejects_legacy_rules_changed(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt.update(
            {
                "status": "stopped",
                "final_type": "rules_changed",
                "reason": "Legacy reason.",
                "ended_at": "2026-09-01T10:05+08:00",
            }
        )

        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(
                attempt,
                project_root=SKILL_ROOT.parent.parent,
            )

        self.assertEqual(context.exception.code, "attempt_invalid_final_type")

    def test_unchanged_instructions_allow_normal_stop_reason(self) -> None:
        current = _validate_execute_instruction_close_state(
            self.task,
            self.attempt,
            self.stopped_request("user_stopped"),
        )

        self.assertEqual(current, self.execute_selection)

    def test_changed_instructions_require_instructions_changed_reason(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt["execute_instructions_sha256"] = "0" * 64

        current = _validate_execute_instruction_close_state(
            self.task,
            attempt,
            self.stopped_request("instructions_changed"),
        )

        self.assertEqual(current, self.execute_selection)
        self.assertEqual(
            _task_status(self.stopped_request("instructions_changed")),
            "blocked",
        )

    def test_rejects_instruction_change_state_mismatch(self) -> None:
        with self.assertRaises(WorkError) as context:
            _validate_execute_instruction_close_state(
                self.task,
                self.attempt,
                self.stopped_request("instructions_changed"),
            )

        self.assertEqual(
            context.exception.code,
            "attempt_close_execute_instructions_state_mismatch",
        )


if __name__ == "__main__":
    unittest.main()
