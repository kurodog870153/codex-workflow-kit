from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.transactions import TransactionErrors, prepare_and_replace
from worklib.foundation.errors import ExitCode, WorkError


ERRORS = TransactionErrors(
    transaction_present=("test_transaction_present", "Transaction already exists."),
    prepare_failed=("test_prepare_failed", "Preparation failed."),
    source_changed=("test_source_changed", "Source changed."),
    replace_failed=("test_replace_failed", "Replacement failed."),
    write_mismatch=("test_write_mismatch", "Stored bytes differ."),
)


class TransactionWriteTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "artifact.json"
        self.temporary = self.root / ".work-test.tmp"
        self.source.write_bytes(b"original")
        self.arguments = {
            "source_path": self.source,
            "source_bytes": b"original",
            "target_bytes": b"updated",
            "temporary_path": self.temporary,
            "stage": "attempt_update_prepared",
            "errors": ERRORS,
        }

    def assert_prepared_error(self, error: WorkError, code: str) -> None:
        self.assertEqual(error.code, code)
        self.assertEqual(
            error.details,
            {
                "path": str(self.temporary),
                "recovery_required": True,
                "transaction_stage": "attempt_update_prepared",
            },
        )

    def test_success_flushes_and_syncs_before_replacing(self) -> None:
        real_replace = os.replace

        def replace(source, target):
            sync.assert_called_once()
            self.assertEqual(self.source.read_bytes(), b"original")
            self.assertEqual(self.temporary.read_bytes(), b"updated")
            return real_replace(source, target)

        with patch(
            "worklib.execution.transactions.os.fsync", wraps=os.fsync
        ) as sync, patch(
            "worklib.execution.transactions.os.replace", side_effect=replace
        ):
            prepare_and_replace(**self.arguments)
        self.assertEqual(self.source.read_bytes(), b"updated")
        self.assertFalse(self.temporary.exists())

    def test_existing_transaction_is_preserved(self) -> None:
        self.temporary.write_bytes(b"existing transaction")
        with self.assertRaises(WorkError) as context:
            prepare_and_replace(**self.arguments)
        self.assertEqual(context.exception.exit_code, ExitCode.LOCK_CONFLICT)
        self.assert_prepared_error(context.exception, "test_transaction_present")
        self.assertIsInstance(context.exception.__cause__, FileExistsError)
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.temporary.read_bytes(), b"existing transaction")

    def test_open_failure_does_not_claim_recovery_without_a_temporary_file(self) -> None:
        failure = PermissionError("cannot create temporary")
        with patch.object(Path, "open", side_effect=failure):
            with self.assertRaises(WorkError) as context:
                prepare_and_replace(**self.arguments)
        self.assertEqual(context.exception.exit_code, ExitCode.IO_FAILURE)
        self.assertEqual(context.exception.code, "test_prepare_failed")
        self.assertEqual(context.exception.details, {"path": str(self.temporary)})
        self.assertIs(context.exception.__cause__, failure)
        self.assertFalse(self.temporary.exists())
        self.assertEqual(self.source.read_bytes(), b"original")

    def test_sync_failure_preserves_prepared_file_and_original_source(self) -> None:
        failure = OSError("sync interrupted")
        with patch("worklib.execution.transactions.os.fsync", side_effect=failure):
            with self.assertRaises(WorkError) as context:
                prepare_and_replace(**self.arguments)
        self.assertEqual(context.exception.exit_code, ExitCode.IO_FAILURE)
        self.assert_prepared_error(context.exception, "test_prepare_failed")
        self.assertIs(context.exception.__cause__, failure)
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.temporary.read_bytes(), b"updated")

    def test_changed_source_is_not_overwritten(self) -> None:
        self.source.write_bytes(b"concurrent change")
        with self.assertRaises(WorkError) as context:
            prepare_and_replace(**self.arguments)
        self.assertEqual(context.exception.exit_code, ExitCode.ARTIFACT_INTEGRITY)
        self.assert_prepared_error(context.exception, "test_source_changed")
        self.assertEqual(self.source.read_bytes(), b"concurrent change")
        self.assertEqual(self.temporary.read_bytes(), b"updated")

    def test_replace_failure_preserves_both_files(self) -> None:
        failure = OSError("replacement interrupted")
        with patch("worklib.execution.transactions.os.replace", side_effect=failure):
            with self.assertRaises(WorkError) as context:
                prepare_and_replace(**self.arguments)
        self.assertEqual(context.exception.exit_code, ExitCode.IO_FAILURE)
        self.assert_prepared_error(context.exception, "test_replace_failed")
        self.assertIs(context.exception.__cause__, failure)
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.temporary.read_bytes(), b"updated")

    def test_readback_mismatch_uses_the_workflow_stage(self) -> None:
        for mismatch_stage in (None, "attempt_update_updated"):
            with self.subTest(mismatch_stage=mismatch_stage):
                self.source.write_bytes(b"original")
                with patch(
                    "worklib.execution.transactions.read_raw",
                    side_effect=[b"original", b"corrupted"],
                ):
                    with self.assertRaises(WorkError) as context:
                        prepare_and_replace(
                            **self.arguments, mismatch_stage=mismatch_stage
                        )
                self.assertEqual(context.exception.exit_code, ExitCode.ARTIFACT_INTEGRITY)
                self.assertEqual(context.exception.code, "test_write_mismatch")
                self.assertEqual(
                    context.exception.details,
                    {
                        "recovery_required": True,
                        "transaction_stage": mismatch_stage or "attempt_update_prepared",
                    },
                )
                self.assertEqual(self.source.read_bytes(), b"updated")
                self.assertFalse(self.temporary.exists())

    def test_read_errors_remain_available_to_workflow_recovery(self) -> None:
        failure = WorkError(
            ExitCode.IO_FAILURE, "file_read_failed", "Cannot read source."
        )
        with patch("worklib.execution.transactions.read_raw", side_effect=failure):
            with self.assertRaises(WorkError) as context:
                prepare_and_replace(**self.arguments)
        self.assertIs(context.exception, failure)
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.temporary.read_bytes(), b"updated")


if __name__ == "__main__":
    unittest.main()
