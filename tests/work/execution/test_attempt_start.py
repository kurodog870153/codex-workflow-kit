from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.execution.attempt_start import _build_attempt, _lock
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)


class ExecuteInstructionAttemptStartTests(unittest.TestCase):
    def preflight(self) -> dict[str, object]:
        return {
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "task_sha256": "a" * 64,
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
                "task_sha256": "a" * 64,
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
            request={},
            original_status="pending",
            attempt_id="ATTEMPT-001",
            started_at="2026-09-01T10:00+08:00",
        )

    def test_builds_attempt_with_instruction_fingerprints(self) -> None:
        attempt = self.attempt()

        self.assertEqual(attempt["task_instructions_sha256"], "b" * 64)
        self.assertEqual(attempt["execute_instructions_sha256"], "c" * 64)
        self.assertEqual(attempt["hierarchy_selection_sha256"], "f" * 64)
        self.assertIsNone(attempt["skill_id"])
        self.assertEqual(attempt["execute_skill_selection_sha256"], "e" * 64)
        self.assertNotIn("task_rules_sha256", attempt)
        self.assertNotIn("execute_rules_sha256", attempt)

    @patch("worklib.execution.attempt_start._load_source_attempt")
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
