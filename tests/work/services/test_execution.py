from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.services.execution import ExecutionService


class ExecutionServiceTests(unittest.TestCase):
    def options(self, root: Path) -> dict[str, object]:
        return {
            "project_root": root,
            "user_config_root": str(root),
            "raw_task_path": "outputs/work/tasks/example/index.json",
            "raw_execution_dir": "outputs/work/executions/example",
            "task_id": "TASK-001",
            "skill_roots": [],
        }

    @patch("worklib.services.execution.require_no_spec_update")
    @patch("worklib.services.execution.execute_preflight", return_value={"status": "valid"})
    @patch("worklib.services.execution.state_writer")
    def test_read_only_operation_does_not_acquire_writer(
        self, writer, preflight, require_no_spec_update
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = ExecutionService().execute(
                "preflight", **self.options(Path(temporary)), confirmed_inputs=[]
            )

        self.assertEqual(result, {"status": "valid"})
        writer.assert_not_called()
        preflight.assert_called_once()
        require_no_spec_update.assert_called_once()

    @patch("worklib.services.execution.require_no_spec_update")
    @patch("worklib.services.execution.begin_record", return_value={"status": "reserved"})
    @patch("worklib.services.execution.load_task_execution_context")
    @patch("worklib.services.execution.state_writer")
    def test_mutation_loads_context_and_acquires_writer(
        self, writer, load_context, begin, require_no_spec_update
    ) -> None:
        writer.return_value.__enter__.return_value = None
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            task = root / "outputs/work/tasks/example/index.json"
            task.parent.mkdir(parents=True)
            task.write_text("{}")
            (root / "outputs/work/executions/example").mkdir(parents=True)

            result = ExecutionService().execute(
                "record-begin", **self.options(root), base_record_id="VAL-001"
            )

        self.assertEqual(result, {"status": "reserved"})
        load_context.assert_called_once()
        writer.assert_called_once()
        begin.assert_called_once()
        require_no_spec_update.assert_called_once()


if __name__ == "__main__":
    unittest.main()
