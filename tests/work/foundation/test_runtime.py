from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.technical.foundation.runtime import MINIMUM_PYTHON, installed_work_root, supported_python


class RuntimePathTests(unittest.TestCase):
    def test_supported_python_requires_314(self) -> None:
        self.assertEqual(MINIMUM_PYTHON, (3, 14))
        self.assertFalse(supported_python((3, 13, 9)))
        self.assertTrue(supported_python((3, 14, 0)))

    def test_installed_work_root_contains_the_runtime_package(self) -> None:
        work_root = installed_work_root()

        self.assertEqual(work_root.name, "work")
        self.assertEqual(work_root / "scripts", SCRIPT_ROOT)
        self.assertTrue((work_root / "SKILL.md").is_file())


if __name__ == "__main__":
    unittest.main()
