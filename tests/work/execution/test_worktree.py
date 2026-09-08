from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.worktree import inspect_execute_worktree


class ExecuteWorktreeTests(unittest.TestCase):
    @patch("worklib.execution.worktree.collect_git_status", return_value=[])
    @patch("worklib.execution.worktree.canonical_sha256", return_value="a" * 64)
    @patch("worklib.execution.worktree.read_raw", return_value=b"task")
    @patch("worklib.execution.worktree.parse_markdown_json_contract")
    @patch("worklib.execution.worktree.execute_preflight")
    def test_forwards_instruction_fingerprints(
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
            "task_path": "outputs/work/tasks/example/task.md",
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
            raw_task_path="outputs/work/tasks/example/task.md",
            raw_execution_dir="outputs/work/executions/example",
            task_id="TASK-001",
        )

        self.assertEqual(result["task_instructions_sha256"], "b" * 64)
        self.assertEqual(result["execute_instructions_sha256"], "c" * 64)
        self.assertNotIn("task_rules_sha256", result)
        self.assertNotIn("execute_rules_sha256", result)


if __name__ == "__main__":
    unittest.main()
