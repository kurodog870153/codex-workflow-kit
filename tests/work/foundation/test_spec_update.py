from __future__ import annotations

import hashlib
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.foundation.spec_update import (
    completion_marker_matches, require_no_spec_update, storage_path,
    transaction_completion_state,
)


class CompletionMarkerTests(unittest.TestCase):
    def test_completion_state_distinguishes_missing_and_corrupt_markers(self):
        raw = b"journal\n"
        marker = hashlib.sha256(raw).hexdigest().encode("ascii") + b"\n"
        self.assertEqual(transaction_completion_state(raw, None), "incomplete")
        self.assertEqual(transaction_completion_state(raw, b"bad\n"), "corrupt")
        self.assertEqual(transaction_completion_state(raw, marker), "completed")

    def test_storage_rejects_hardlink_aliases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original = root / "original"
            original.write_bytes(b"preserve")
            alias = root / "alias"
            os.link(original, alias)
            with self.assertRaises(WorkError) as caught:
                storage_path(root, "alias")
            self.assertEqual(caught.exception.code, "spec_update_alias")
            self.assertEqual(original.read_bytes(), b"preserve")

    def test_storage_rejects_symbolic_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original = root / "original"
            original.write_bytes(b"preserve")
            link = root / "link"
            try:
                link.symlink_to(original)
            except OSError as error:
                if getattr(error, "winerror", None) == 1314:
                    self.skipTest("Windows symbolic-link privilege is unavailable.")
                raise
            with self.assertRaises(WorkError) as caught:
                storage_path(root, "link")
            self.assertEqual(caught.exception.code, "spec_update_link")
            self.assertEqual(original.read_bytes(), b"preserve")

    def test_exact_raw_bytes_and_marker_encoding(self):
        for raw in (b"", b"{}\n", b"{}\r\n", b"\xef\xbb\xbf{}\n", b"\xff"):
            marker = hashlib.sha256(raw).hexdigest().encode("ascii") + b"\n"
            with self.subTest(raw=raw):
                self.assertTrue(completion_marker_matches(raw, marker))
                for invalid in (b"", marker[:-1], marker + b"\n", marker[:-1] + b"\r\n", marker.upper()):
                    self.assertFalse(completion_marker_matches(raw, invalid))
                self.assertFalse(completion_marker_matches(raw + b"\n", marker))

    def test_guard_preserves_pending_policy_for_both_record_types(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "execution").mkdir()
            for prefix in ("spec-update", "task-repair"):
                record = root / "execution" / f".work-{prefix}-example.json"
                record.write_bytes(b"{}\n")
                marker = Path(str(record) + ".done")
                with self.assertRaises(WorkError) as caught:
                    require_no_spec_update(root, "execution")
                self.assertEqual(caught.exception.code, "spec_update_pending")
                marker.write_bytes(b"invalid")
                with self.assertRaises(WorkError) as caught:
                    require_no_spec_update(root, "execution")
                self.assertEqual(caught.exception.code, "spec_update_pending")
                marker.write_bytes(hashlib.sha256(record.read_bytes()).hexdigest().encode("ascii") + b"\n")
                require_no_spec_update(root, "execution")

    def test_guard_does_not_reclassify_read_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "execution").mkdir()
            (root / "execution" / ".work-spec-update-example.json").write_bytes(b"{}\n")
            with patch.object(Path, "read_bytes", side_effect=OSError("read failed")):
                with self.assertRaises(OSError):
                    require_no_spec_update(root, "execution")
