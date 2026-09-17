from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.infrastructure.writer_lock import require_idle_writer, state_writer
from worklib.foundation.errors import WorkError


class WriterLockTests(unittest.TestCase):
    def test_idle_probe_does_not_create_storage(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            require_idle_writer(root, "execution")
            self.assertEqual(list(root.iterdir()), [])

    def test_active_writer_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "execution").mkdir()
            with state_writer(root, "execution"):
                with self.assertRaises(WorkError) as context:
                    require_idle_writer(root, "execution")
            self.assertEqual(context.exception.code, "work_state_writer_busy")


if __name__ == "__main__":
    unittest.main()
