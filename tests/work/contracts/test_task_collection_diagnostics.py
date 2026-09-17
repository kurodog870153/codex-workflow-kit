from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts.test_task_collection import TaskCollectionTests
from worklib.contracts.task_diagnostics import diagnose_task_collection
from worklib.contracts.task_index import render_task_index_contract


class TaskCollectionDiagnosticsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = TaskCollectionTests("test_loads_complete_v2_collection_and_v1_artifact")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.index_path = self.fixture.index_path

    def diagnose(self) -> dict[str, object]:
        return diagnose_task_collection(
            self.root, str(self.root), self.index_path
        )

    def test_valid_collection_passes_with_explicit_execution_skip(self) -> None:
        result = self.diagnose()
        checks = {check["name"]: check for check in result["checks"]}

        self.assertEqual(result["status"], "valid")
        self.assertTrue(result["normal_use_allowed"])
        self.assertEqual(checks["index:contract"]["status"], "passed")
        self.assertEqual(checks["item:TASK-001:contract"]["status"], "passed")
        self.assertEqual(checks["collection:contract"]["status"], "passed")
        self.assertEqual(checks["execution_index_binding"]["status"], "not_checked")
        self.assertEqual(
            checks["execution_index_binding"]["requires"],
            ["v2_execution_index_contract"],
        )

    def test_malformed_index_skips_dependent_checks(self) -> None:
        (self.root / self.index_path).write_bytes(b"{")

        result = self.diagnose()
        checks = {check["name"]: check for check in result["checks"]}

        self.assertEqual(result["status"], "blocked")
        self.assertEqual(checks["index:json"]["status"], "failed")
        self.assertEqual(checks["index:contract"]["status"], "not_checked")
        self.assertEqual(checks["collection:contract"]["status"], "not_checked")

    def test_missing_item_reports_read_failure_and_collection_skip(self) -> None:
        item_path = (self.root / self.index_path).parent / "tasks" / "TASK-001.json"
        item_path.unlink()

        result = self.diagnose()
        checks = {check["name"]: check for check in result["checks"]}

        self.assertEqual(checks["item:TASK-001:file"]["status"], "failed")
        self.assertEqual(checks["item:TASK-001:contract"]["status"], "not_checked")
        self.assertEqual(checks["items:directory"]["status"], "failed")
        self.assertEqual(checks["collection:contract"]["status"], "not_checked")

    def test_hash_mismatch_does_not_hide_item_contract(self) -> None:
        index = copy.deepcopy(self.fixture.index)
        index["tasks"][0]["canonical_sha256"] = "0" * 64  # type: ignore[index]
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))

        result = self.diagnose()
        checks = {check["name"]: check for check in result["checks"]}

        self.assertEqual(checks["item:TASK-001:contract"]["status"], "passed")
        self.assertEqual(checks["item:TASK-001:fingerprint"]["status"], "failed")
        self.assertEqual(checks["collection:contract"]["status"], "failed")

    def test_orphan_and_broken_item_are_both_reported(self) -> None:
        directory = (self.root / self.index_path).parent / "tasks"
        (directory / "TASK-001.json").write_bytes(b"{")
        (directory / "TASK-999.json").write_bytes(b"{}\n")

        result = self.diagnose()
        checks = {check["name"]: check for check in result["checks"]}
        issue_codes = {issue["code"] for issue in result["issues"]}

        self.assertEqual(checks["item:TASK-001:json"]["status"], "failed")
        self.assertEqual(checks["items:directory"]["status"], "failed")
        self.assertIn("invalid_json_contract", issue_codes)
        self.assertIn("task_collection_directory_mismatch", issue_codes)
        self.assertEqual(checks["collection:contract"]["status"], "not_checked")


if __name__ == "__main__":
    unittest.main()
