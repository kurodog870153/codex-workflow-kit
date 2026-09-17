from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.infrastructure.command_receipt_storage import write_command_receipt


class CommandReceiptStorageTests(unittest.TestCase):
    def test_writes_flushes_and_syncs_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "started.json"
            with patch(
                "worklib.infrastructure.command_receipt_storage.os.fsync",
                wraps=os.fsync,
            ) as sync:
                write_command_receipt(path, b"{}\n")
            sync.assert_called_once()
            self.assertEqual(path.read_bytes(), b"{}\n")

    def test_existing_receipt_is_never_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "started.json"
            path.write_bytes(b"preserved")
            with self.assertRaises(FileExistsError):
                write_command_receipt(path, b"replacement")
            self.assertEqual(path.read_bytes(), b"preserved")


if __name__ == "__main__":
    unittest.main()
