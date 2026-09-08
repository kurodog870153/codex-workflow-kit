from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.attempt_start_transactions import (
    raise_transaction_error,
    replace_index,
    transaction_path,
    transaction_stage,
    validate_snapshot,
    write_exclusive,
)
from worklib.foundation.errors import ExitCode, WorkError


class AttemptStartTransactionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.directory = Path(self.temporary_directory.name)

    def test_builds_transaction_path(self) -> None:
        self.assertEqual(
            transaction_path(
                self.directory,
                task_id="TASK-001",
                attempt_id="ATTEMPT-001",
                stage="lock",
            ),
            self.directory
            / ".work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp",
        )

    def test_exclusive_write_preserves_existing_target(self) -> None:
        path = self.directory / "attempt.json"
        write_exclusive(path, b"first", label="Attempt document")

        with self.assertRaises(WorkError) as context:
            write_exclusive(path, b"second", label="Attempt document")

        self.assertEqual(path.read_bytes(), b"first")
        self.assertEqual(context.exception.code, "attempt_start_target_exists")

    @patch("worklib.execution.attempt_start_transactions.validate_execution_index")
    @patch(
        "worklib.execution.attempt_start_transactions.render_execution_index",
        return_value=b"new",
    )
    def test_replaces_unchanged_index(self, _mocked_render, mocked_validate) -> None:
        index_path = self.directory / "index.json"
        temporary_path = self.directory / "index.tmp"
        index_path.write_bytes(b"old")

        stored = replace_index(
            index_path=index_path,
            expected_current=b"old",
            target={"overall_status": "in_progress"},
            temporary_path=temporary_path,
            allow_existing_temporary=False,
        )

        self.assertEqual(stored, b"new")
        self.assertEqual(index_path.read_bytes(), b"new")
        self.assertFalse(temporary_path.exists())
        self.assertEqual(mocked_validate.call_count, 2)

    @patch("worklib.execution.attempt_start_transactions.read_index")
    def test_transaction_stage_uses_recovery_precedence(self, mocked_read) -> None:
        index_path = self.directory / "index.json"
        attempt_path = self.directory / "attempt.json"
        lock_temporary = self.directory / "lock.tmp"
        started_temporary = self.directory / "started.tmp"
        mocked_read.return_value = (b"index", {"lock": {"attempt_id": "ATTEMPT-001"}})

        self.assertEqual(
            transaction_stage(
                index_path=index_path,
                attempt_path=attempt_path,
                lock_temporary=lock_temporary,
                started_temporary=started_temporary,
                attempt_id="ATTEMPT-001",
            ),
            "lock_installed",
        )
        attempt_path.write_bytes(b"attempt")
        self.assertEqual(
            transaction_stage(
                index_path=index_path,
                attempt_path=attempt_path,
                lock_temporary=lock_temporary,
                started_temporary=started_temporary,
                attempt_id="ATTEMPT-001",
            ),
            "attempt_created",
        )
        started_temporary.write_bytes(b"started")
        self.assertEqual(
            transaction_stage(
                index_path=index_path,
                attempt_path=attempt_path,
                lock_temporary=lock_temporary,
                started_temporary=started_temporary,
                attempt_id="ATTEMPT-001",
            ),
            "started_index_prepared",
        )

    @patch(
        "worklib.execution.attempt_start_transactions.transaction_stage",
        return_value="attempt_created",
    )
    def test_transaction_error_adds_recovery_metadata(self, _mocked_stage) -> None:
        original = WorkError(ExitCode.IO_FAILURE, "write_failed", "Write failed.")

        with self.assertRaises(WorkError) as context:
            raise_transaction_error(
                original,
                index_path=self.directory / "index.json",
                attempt_path=self.directory / "attempt.json",
                lock_temporary=self.directory / "lock.tmp",
                started_temporary=self.directory / "started.tmp",
                attempt_id="ATTEMPT-001",
            )

        self.assertEqual(context.exception.code, "write_failed")
        self.assertTrue(context.exception.details["recovery_required"])
        self.assertEqual(context.exception.details["attempt_id"], "ATTEMPT-001")
        self.assertEqual(
            context.exception.details["transaction_stage"],
            "attempt_created",
        )

    @patch(
        "worklib.execution.attempt_start_transactions.worktree_snapshot_sha256",
        return_value="actual",
    )
    @patch(
        "worklib.execution.attempt_start_transactions.collect_git_status",
        return_value=[],
    )
    def test_snapshot_mismatch_reports_expected_and_actual(
        self,
        _mocked_status,
        _mocked_snapshot,
    ) -> None:
        with self.assertRaises(WorkError) as context:
            validate_snapshot(
                project_root=self.directory,
                execution_dir="outputs/work/executions/example",
                expected="expected",
            )

        self.assertEqual(
            context.exception.code,
            "attempt_start_worktree_snapshot_changed",
        )
        self.assertEqual(context.exception.details["expected"], "expected")
        self.assertEqual(context.exception.details["actual"], "actual")


if __name__ == "__main__":
    unittest.main()
