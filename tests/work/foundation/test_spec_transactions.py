from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation import spec_transactions as transactions
from worklib.foundation.errors import ExitCode, WorkError


class SpecificationTransactionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = self.root / "artifact.json"
        self.temporary = self.root / "prepared.tmp"

    def assert_code(self, expected, operation):
        with self.assertRaises(WorkError) as caught:
            operation()
        self.assertEqual(caught.exception.code, expected)
        self.assertEqual(caught.exception.exit_code, ExitCode.ARTIFACT_INTEGRITY)

    def replace(self, **kwargs):
        transactions.replace_checked(self.path, b"old", b"new", self.temporary, **kwargs)

    def test_exclusive_write_syncs_and_preserves_existing_bytes(self):
        with patch.object(transactions.os, "fsync", wraps=os.fsync) as sync:
            transactions.write_exclusive(self.path, b"original")
        sync.assert_called_once()
        with self.assertRaises(FileExistsError):
            transactions.write_exclusive(self.path, b"replacement")
        self.assertEqual(self.path.read_bytes(), b"original")

    def test_recovery_creates_or_appends_only_matching_suffix(self):
        transactions.complete_write(self.path, b"pre")
        transactions.complete_write(self.path, b"prefix")
        self.assertEqual(self.path.read_bytes(), b"prefix")
        with patch.object(transactions.os, "fsync") as sync:
            transactions.complete_write(self.path, b"prefix")
        sync.assert_not_called()
        for conflicting in (b"other", b"pre"):
            self.assert_code("spec_update_partial_conflict", lambda: transactions.complete_write(self.path, conflicting))
            self.assertEqual(self.path.read_bytes(), b"prefix")

    def test_publish_and_recovery_install_exact_bytes(self):
        self.path.write_bytes(b"old")
        self.replace()
        self.assertEqual(self.path.read_bytes(), b"new")
        self.assertFalse(self.temporary.exists())
        self.path.write_bytes(b"old")
        self.temporary.write_bytes(b"n")
        self.replace(recover=True)
        self.assertEqual(self.path.read_bytes(), b"new")
        self.assertFalse(self.temporary.exists())

    def test_changed_temporary_preserves_both_files(self):
        self.path.write_bytes(b"old")
        self.temporary.write_bytes(b"unknown")
        self.assert_code("spec_update_temporary_changed", self.replace)
        self.assertEqual(self.path.read_bytes(), b"old")
        self.assertEqual(self.temporary.read_bytes(), b"unknown")

    def test_changed_source_preserves_source_and_prepared_evidence(self):
        self.path.write_bytes(b"external")
        self.assert_code("spec_update_concurrent_change", self.replace)
        self.assertEqual(self.path.read_bytes(), b"external")
        self.assertEqual(self.temporary.read_bytes(), b"new")

    def test_failed_replace_preserves_both_files_and_io_error(self):
        self.path.write_bytes(b"old")
        with patch.object(transactions.os, "replace", side_effect=OSError("interrupted")):
            with self.assertRaises(OSError):
                self.replace()
        self.assertEqual(self.path.read_bytes(), b"old")
        self.assertEqual(self.temporary.read_bytes(), b"new")

    def test_post_publication_mismatch_is_reported(self):
        self.path.write_bytes(b"old")
        original_replace = os.replace

        def changed(source, target):
            original_replace(source, target)
            Path(target).write_bytes(b"external")

        with patch.object(transactions.os, "replace", side_effect=changed):
            self.assert_code("spec_update_write_mismatch", self.replace)
        self.assertEqual(self.path.read_bytes(), b"external")
