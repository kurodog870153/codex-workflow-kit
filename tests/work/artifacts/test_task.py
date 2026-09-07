from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.task import create_task_artifacts
from worklib.foundation.errors import ExitCode, WorkError


class TaskArtifactTests(unittest.TestCase):
    def test_create_writes_task_and_initial_index_as_one_workflow(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project_root = Path(temporary).resolve()
            task_path = project_root / "outputs" / "task.md"
            execution_path = project_root / "outputs" / "execution"
            validation = {
                "requirement_id": "example",
                "spec_id": "TASK-SPEC-001",
                "task_sha256": "a" * 64,
            }
            initial_index = {"schema": "work-execution-index/v1"}
            inputs = (
                {"artifacts": {}},
                validation,
                b"canonical task",
                initial_index,
                b"canonical index",
                "outputs/task.md",
                task_path,
                "outputs/execution",
                execution_path,
            )
            stored_index = {"index_sha256": "b" * 64}
            with patch(
                "worklib.artifacts.task._task_create_inputs",
                return_value=inputs,
            ), patch(
                "worklib.artifacts.task._validate_created_pair",
                return_value=(validation, stored_index),
            ):
                result = create_task_artifacts(
                    b"request",
                    source="test",
                    raw_plan_path="outputs/plan.md",
                    raw_task_path="outputs/task.md",
                    raw_execution_dir="outputs/execution",
                    project_root=project_root,
                    user_config_root=temporary,
                )
                self.assertEqual(task_path.read_bytes(), b"canonical task")
                self.assertEqual(
                    (execution_path / "index.md").read_bytes(), b"canonical index"
                )
                self.assertEqual(result["status"], "created")

                with self.assertRaises(WorkError) as context:
                    create_task_artifacts(
                        b"request",
                        source="test",
                        raw_plan_path="outputs/plan.md",
                        raw_task_path="outputs/task.md",
                        raw_execution_dir="outputs/execution",
                        project_root=project_root,
                        user_config_root=temporary,
                    )
                self.assertEqual(context.exception.exit_code, ExitCode.WORKFLOW_STATE)
                self.assertEqual(context.exception.code, "task_create_target_exists")
                self.assertEqual(task_path.read_bytes(), b"canonical task")
                self.assertEqual(
                    (execution_path / "index.md").read_bytes(), b"canonical index"
                )


if __name__ == "__main__":
    unittest.main()
