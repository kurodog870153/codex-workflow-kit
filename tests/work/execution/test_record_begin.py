from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.record_begin import _write_lock_update, begin_record
from worklib.foundation.errors import ExitCode, WorkError


class RecordBeginTests(unittest.TestCase):
    def test_rejects_invalid_base_record_id_before_reading_artifacts(self) -> None:
        with self.assertRaises(WorkError) as context:
            begin_record(
                project_root=Path.cwd(),
                user_config_root=str(Path.cwd()),
                raw_task_path="task.json",
                raw_execution_dir="execution",
                task_id="TASK-001",
                base_record_id="CMD-1",
            )

        self.assertEqual(context.exception.exit_code, ExitCode.CONTRACT)
        self.assertEqual(context.exception.code, "record_begin_invalid_base_record_id")
        self.assertEqual(context.exception.details["record_id"], "CMD-1")

    @patch("worklib.execution.record_begin.validate_execution_index")
    @patch("worklib.execution.record_begin.render_execution_index", return_value=b"new")
    @patch("worklib.execution.record_begin.read_raw", side_effect=[b"old", b"new"])
    def test_lock_update_replaces_unchanged_index_atomically(
        self,
        mocked_read,
        mocked_render,
        mocked_validate,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            index_path = directory / "index.json"
            temporary_path = directory / ".work-record-begin.tmp"
            index_path.write_bytes(b"old")

            _write_lock_update(
                index_path=index_path,
                index_raw=b"old",
                target_index={"lock": {"record_id": "CMD-001"}},
                temporary_path=temporary_path,
            )

            self.assertEqual(index_path.read_bytes(), b"new")
            self.assertFalse(temporary_path.exists())

        mocked_render.assert_called_once_with(
            {"lock": {"record_id": "CMD-001"}}
        )
        self.assertEqual(mocked_read.call_count, 2)
        self.assertEqual(mocked_validate.call_count, 2)


if __name__ == "__main__":
    unittest.main()
