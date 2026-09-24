from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "run_work.py"
SPEC = importlib.util.spec_from_file_location("run_work", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class TestRunnerTests(unittest.TestCase):
    def test_full_discovery_has_unique_ids(self) -> None:
        ids = runner.test_ids(runner.discover())
        self.assertTrue(ids)
        self.assertEqual(len(ids), len(set(ids)))

    def test_selected_directory_only_loads_its_tests(self) -> None:
        ids = runner.test_ids(runner.discover("integration", "test_task_collection_io.py"))
        self.assertEqual(len(ids), 3)
        self.assertTrue(all("test_task_collection_io" in test_id for test_id in ids))

    def test_duplicate_ids_are_rejected(self) -> None:
        case = self.__class__("test_duplicate_ids_are_rejected")
        suite = unittest.TestSuite([case, case])
        with patch.object(unittest.TestLoader, "discover", return_value=suite):
            with self.assertRaisesRegex(ValueError, "duplicate test IDs"):
                runner.discover()

    def test_path_cannot_escape_test_root(self) -> None:
        with self.assertRaises(ValueError):
            runner.discover("..")


if __name__ == "__main__":
    unittest.main()
