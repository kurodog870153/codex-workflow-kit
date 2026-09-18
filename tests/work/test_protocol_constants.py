from __future__ import annotations

import sys
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.protocol import SHA256_PATTERN, WORKFLOW_MODES


class ProtocolConstantTests(unittest.TestCase):
    def test_shared_protocol_values_are_stable(self) -> None:
        self.assertEqual(SHA256_PATTERN, r"^[0-9a-f]{64}$")
        self.assertEqual(WORKFLOW_MODES, ("plan", "task", "execute"))

    def test_workflow_modes_are_immutable(self) -> None:
        self.assertIsInstance(WORKFLOW_MODES, tuple)


if __name__ == "__main__":
    unittest.main()
