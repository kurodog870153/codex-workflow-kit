from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.attempt import canonicalize_attempt_contract
from worklib.foundation.errors import WorkError
from worklib.execution.attempt_start import _build_attempt, _lock
from worklib.execution.worktree import inspect_execute_worktree
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

    def test_attempt_contract_rejects_legacy_rule_fingerprints(self) -> None:
        attempt = copy.deepcopy(self.attempt())
        attempt["task_rules_sha256"] = attempt.pop("task_instructions_sha256")
        attempt["execute_rules_sha256"] = attempt.pop(
            "execute_instructions_sha256"
        )

        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)

        self.assertEqual(context.exception.code, "attempt_invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["execute_instructions_sha256", "task_instructions_sha256"],
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["execute_rules_sha256", "task_rules_sha256"],
        )

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

    @patch("worklib.execution.worktree.collect_git_status", return_value=[])
    @patch("worklib.execution.worktree.canonical_sha256", return_value="a" * 64)
    @patch("worklib.execution.worktree.read_raw", return_value=b"task")
    @patch("worklib.execution.worktree.parse_markdown_json_contract")
    @patch("worklib.execution.worktree.execute_preflight")
    def test_worktree_forwards_instruction_fingerprints(
        self,
        mocked_preflight,
        mocked_parse,
        _mocked_read,
        _mocked_sha256,
        _mocked_status,
    ) -> None:
        mocked_preflight.return_value = {
            "requirement_id": "example",
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "task_sha256": "a" * 64,
            "task_instructions_sha256": "b" * 64,
            "execute_instructions_sha256": "c" * 64,
            "execute_skill_selection": {"selection_sha256": "e" * 64},
            "task_status": "pending",
            "task_path": "outputs/work/tasks/example.md",
            "index_sha256": "d" * 64,
            "execution_dir": "outputs/work/executions/example",
            "dependencies": [],
        }
        mocked_parse.return_value = (
            "Example TASK",
            {"tasks": [{"id": "TASK-001"}]},
        )

        result = inspect_execute_worktree(
            project_root=REPO_ROOT,
            user_config_root=str(REPO_ROOT),
            raw_task_path="outputs/work/tasks/example.md",
            raw_execution_dir="outputs/work/executions/example",
            task_id="TASK-001",
        )

        self.assertEqual(result["task_instructions_sha256"], "b" * 64)
        self.assertEqual(result["execute_instructions_sha256"], "c" * 64)
        self.assertNotIn("task_rules_sha256", result)
        self.assertNotIn("execute_rules_sha256", result)


if __name__ == "__main__":
    unittest.main()
