from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.correction_transactions import (
    consume_temporary,
    install_exclusive,
    prepare_file,
    replace_file,
)
from worklib.foundation.errors import WorkError


class CorrectionTransactionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.directory = Path(self.temporary_directory.name)

    def test_prepares_and_replaces_unchanged_source(self) -> None:
        target = self.directory / "index.json"
        temporary = self.directory / "index.tmp"
        target.write_bytes(b"old")

        prepare_file(temporary, b"new", stage="index_prepared")
        replace_file(
            temporary,
            target,
            expected=b"new",
            source_bytes=b"old",
            stage="index_updated",
        )

        self.assertEqual(target.read_bytes(), b"new")
        self.assertFalse(temporary.exists())

    def test_prepare_preserves_existing_transaction(self) -> None:
        temporary = self.directory / "index.tmp"
        temporary.write_bytes(b"existing")

        with self.assertRaises(WorkError) as context:
            prepare_file(temporary, b"new", stage="index_prepared")

        self.assertEqual(context.exception.code, "correction_create_transaction_present")
        self.assertTrue(context.exception.details["recovery_required"])
        self.assertEqual(context.exception.details["transaction_stage"], "index_prepared")

    def test_replace_rejects_changed_source(self) -> None:
        target = self.directory / "index.json"
        temporary = self.directory / "index.tmp"
        target.write_bytes(b"changed")
        temporary.write_bytes(b"new")

        with self.assertRaises(WorkError) as context:
            replace_file(
                temporary,
                target,
                expected=b"new",
                source_bytes=b"old",
                stage="index_updated",
            )

        self.assertEqual(context.exception.code, "correction_create_source_changed")
        self.assertEqual(temporary.read_bytes(), b"new")
        self.assertEqual(target.read_bytes(), b"changed")

    def test_installs_immutable_target_and_consumes_temporary(self) -> None:
        target = self.directory / "correction.json"
        temporary = self.directory / "correction.tmp"
        temporary.write_bytes(b"correction")

        install_exclusive(
            temporary,
            target,
            expected=b"correction",
            stage="artifact_installed",
        )

        self.assertEqual(target.read_bytes(), b"correction")
        self.assertFalse(temporary.exists())

    def test_exclusive_install_rejects_existing_target(self) -> None:
        target = self.directory / "correction.json"
        temporary = self.directory / "correction.tmp"
        target.write_bytes(b"existing")
        temporary.write_bytes(b"correction")

        with self.assertRaises(WorkError) as context:
            install_exclusive(
                temporary,
                target,
                expected=b"correction",
                stage="artifact_installed",
            )

        self.assertEqual(context.exception.code, "correction_create_target_exists")

    @patch("worklib.execution.correction_transactions.os.unlink", side_effect=OSError)
    def test_consume_failure_reports_recovery_stage(self, _mocked_unlink) -> None:
        temporary = self.directory / "correction.tmp"
        temporary.write_bytes(b"correction")

        with self.assertRaises(WorkError) as context:
            consume_temporary(temporary, stage="artifact_installed")

        self.assertEqual(
            context.exception.code,
            "correction_create_temporary_consume_failed",
        )
        self.assertTrue(context.exception.details["recovery_required"])
        self.assertEqual(
            context.exception.details["transaction_stage"],
            "artifact_installed",
        )


if __name__ == "__main__":
    unittest.main()
