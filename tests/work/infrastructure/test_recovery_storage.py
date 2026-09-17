from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import ExitCode, WorkError
from worklib.infrastructure.recovery_storage import (
    install_recovery_target,
    prepare_recovery_target,
)


class RecoveryStorageTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_prepare_is_durable_and_idempotent(self) -> None:
        path = self.root / "prepared.tmp"
        with patch(
            "worklib.infrastructure.recovery_storage.os.fsync", wraps=os.fsync
        ) as sync:
            prepare_recovery_target(path, b"expected")
        sync.assert_called_once()
        prepare_recovery_target(path, b"expected")
        self.assertEqual(path.read_bytes(), b"expected")

    def test_prepare_rejects_conflicting_existing_bytes(self) -> None:
        path = self.root / "prepared.tmp"
        path.write_bytes(b"different")
        with self.assertRaises(WorkError) as context:
            prepare_recovery_target(path, b"expected")
        self.assertEqual(
            context.exception.code,
            "execution_recovery_prepared_bytes_mismatch",
        )
        self.assertEqual(path.read_bytes(), b"different")

    def test_install_rejects_changed_source_before_replacement(self) -> None:
        temporary = self.root / "prepared.tmp"
        target = self.root / "target.json"
        temporary.write_bytes(b"expected")
        target.write_bytes(b"changed")
        with patch("worklib.infrastructure.recovery_storage.os.replace") as replace:
            with self.assertRaises(WorkError) as context:
                install_recovery_target(
                    temporary,
                    target,
                    expected=b"expected",
                    source_bytes=b"original",
                    stage="index_update",
                )
        self.assertEqual(context.exception.code, "execution_recovery_source_changed")
        replace.assert_not_called()

    def test_replace_failure_preserves_recovery_stage(self) -> None:
        temporary = self.root / "prepared.tmp"
        target = self.root / "target.json"
        temporary.write_bytes(b"expected")
        target.write_bytes(b"original")
        failure = OSError("interrupted")
        with patch(
            "worklib.infrastructure.recovery_storage.os.replace",
            side_effect=failure,
        ):
            with self.assertRaises(WorkError) as context:
                install_recovery_target(
                    temporary,
                    target,
                    expected=b"expected",
                    source_bytes=b"original",
                    stage="index_update",
                )
        self.assertEqual(context.exception.exit_code, ExitCode.IO_FAILURE)
        self.assertEqual(context.exception.code, "execution_recovery_replace_failed")
        self.assertEqual(context.exception.details["transaction_stage"], "index_update")
        self.assertTrue(temporary.exists())
        self.assertEqual(target.read_bytes(), b"original")


if __name__ == "__main__":
    unittest.main()
