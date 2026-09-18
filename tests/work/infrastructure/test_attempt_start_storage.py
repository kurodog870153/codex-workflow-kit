from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.technical.infrastructure.attempt_start_storage import (
    replace_index,
    transaction_path,
    write_exclusive,
)
from worklib.models.common.errors import WorkError


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

    def test_replaces_unchanged_index(self) -> None:
        index_path = self.directory / "index.json"
        temporary_path = self.directory / "index.tmp"
        index_path.write_bytes(b"old")

        stored = replace_index(
            index_path=index_path,
            expected_current=b"old",
            rendered=b"new",
            temporary_path=temporary_path,
            allow_existing_temporary=False,
        )

        self.assertEqual(stored, b"new")
        self.assertEqual(index_path.read_bytes(), b"new")
        self.assertFalse(temporary_path.exists())

if __name__ == "__main__":
    unittest.main()
