from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.business_services.execution import ExecutionService
from worklib.orchestration.execution import ExecutionCapabilities


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

    @patch("worklib.business_services.execution.workflow.require_no_spec_update")
    @patch("worklib.orchestration.execution.ExecutionCapabilities.execute_preflight", return_value={"status": "valid"})
    @patch("worklib.business_services.execution.workflow.state_writer")
    def test_read_only_operation_does_not_acquire_writer(
        self, writer, preflight, require_no_spec_update
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = ExecutionService(ExecutionCapabilities).execute(
                "preflight", **self.options(Path(temporary)), confirmed_inputs=[]
            )

        self.assertEqual(result, {"status": "valid"})
        writer.assert_not_called()
        preflight.assert_called_once()
        require_no_spec_update.assert_called_once()

    @patch("worklib.business_services.execution.workflow.require_no_spec_update")
    @patch("worklib.orchestration.execution.ExecutionCapabilities.record_execution_deviation", return_value={"status": "recorded"})
    @patch("worklib.orchestration.execution.ExecutionCapabilities.load_task_execution_context")
    @patch("worklib.business_services.execution.workflow.state_writer")
    def test_deviation_record_acquires_writer_and_passes_approval(
        self, writer, _load_context, record, _require_no_spec_update
    ) -> None:
        writer.return_value.__enter__.return_value = None
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            task = root / "outputs/work/tasks/example/index.json"
            task.parent.mkdir(parents=True)
            task.write_text("{}")
            (root / "outputs/work/executions/example").mkdir(parents=True)
            result = ExecutionService(ExecutionCapabilities).execute(
                "deviation-record", **self.options(root), raw_request=b"{}",
                source="test", approved_sha256="a" * 64,
                authorization_evidence="User approved.",
            )
        self.assertEqual(result, {"status": "recorded"})
        writer.assert_called_once()
        record.assert_called_once()

    @patch("worklib.business_services.execution.workflow.require_no_spec_update")
    @patch("worklib.orchestration.execution.ExecutionCapabilities.prepare_semantic_execution_deviation", return_value={"status": "preview"})
    @patch("worklib.business_services.execution.workflow.state_writer")
    def test_semantic_deviation_prepare_does_not_acquire_writer(
        self, writer, prepare, require_no_spec_update
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(ExecutionCapabilities, "prepare_semantic_execution_deviation", prepare):
                result = ExecutionService(ExecutionCapabilities).execute(
                    "deviation-prepare-semantic", **self.options(Path(temporary)),
                    raw_request=b"{}", source="test",
                )
        self.assertEqual(result, {"status": "preview"})
        writer.assert_not_called()
        prepare.assert_called_once()

    @patch("worklib.business_services.execution.workflow.require_no_spec_update")
    @patch("worklib.orchestration.execution.ExecutionCapabilities.begin_record", return_value={"status": "reserved"})
    @patch("worklib.orchestration.execution.ExecutionCapabilities.load_task_execution_context")
    @patch("worklib.business_services.execution.workflow.state_writer")
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

            result = ExecutionService(ExecutionCapabilities).execute(
                "record-begin", **self.options(root), base_record_id="VAL-001"
            )

        self.assertEqual(result, {"status": "reserved"})
        load_context.assert_called_once()
        writer.assert_called_once()
        begin.assert_called_once()
        require_no_spec_update.assert_called_once()

    @patch("worklib.business_services.execution.workflow.require_no_spec_update")
    @patch("worklib.orchestration.execution.ExecutionCapabilities.start_attempt", return_value={"status": "started"})
    @patch("worklib.orchestration.execution.ExecutionCapabilities.load_task_execution_context")
    @patch("worklib.business_services.execution.workflow.state_writer")
    def test_attempt_start_passes_prelock_context_into_writer(
        self, writer, load_context, start, _require_no_spec_update
    ) -> None:
        writer.return_value.__enter__.return_value = None
        snapshot = {"contract": {"tasks": [{"id": "TASK-001"}]}}
        load_context.return_value = snapshot
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            task = root / "outputs/work/tasks/example/index.json"
            task.parent.mkdir(parents=True)
            task.write_text("{}")
            (root / "outputs/work/executions/example").mkdir(parents=True)
            result = ExecutionService(ExecutionCapabilities).execute(
                "attempt-start", **self.options(root), raw_request=b"{}", source="test",
            )
        self.assertEqual(result, {"status": "started"})
        load_context.assert_called_once()
        writer.assert_called_once()
        self.assertIs(start.call_args.kwargs["_prevalidated_context"], snapshot)


if __name__ == "__main__":
    unittest.main()
