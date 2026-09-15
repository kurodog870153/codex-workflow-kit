from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts.test_task_collection import TaskCollectionTests
from worklib.artifacts.task_collection import load_task_collection
from worklib.artifacts.task_repair import repair_task
from worklib.artifacts.task_repair_prepare import prepare_task_repair
from worklib.contracts.execution_index import build_initial_execution_index, render_execution_index
from worklib.contracts.task_item import render_task_item_contract, validate_task_item_contract
from worklib.foundation import spec_transactions
from worklib.foundation.errors import WorkError


class TaskRepairV2Tests(unittest.TestCase):
    def setUp(self):
        fixture = TaskCollectionTests("test_loads_complete_v2_collection_and_v1_artifact")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.root = fixture.root
        self.artifacts = fixture.index["artifacts"]
        validation = load_task_collection(self.root, str(self.root), fixture.index_path)
        execution = build_initial_execution_index(validation["logical_contract"], validation)
        execution_path = self.root / self.artifacts["execution"] / "index.json"
        execution_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.write_bytes(render_execution_index(execution))

    def prepare_request(self, *, decisions=None):
        request = {
            "schema": "work-task-repair-prepare-request/v2", "stage": "complete",
            "requirement_id": "example", "artifacts": self.artifacts,
            "decisions": decisions or [{"location": "/", "decision": "Use the reviewed candidate."}],
            "task_index": copy.deepcopy(self.fixture.index),
            "task_items": {"TASK-001": copy.deepcopy(self.fixture.item)},
        }
        return prepare_task_repair(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root))

    def run_repair(self, prepared, operation="validate", approval=None):
        return repair_task(json.dumps(prepared["request"]).encode(), project_root=self.root,
                           user_config_root=str(self.root), operation=operation, approved_sha256=approval)

    def test_repairs_item_and_index_fingerprint_with_multi_file_transaction(self):
        item = copy.deepcopy(self.fixture.item)
        item["title"] = "corrupt but reviewed"
        item_path = self.root / self.fixture.index_path
        item_path = item_path.parent / "tasks" / "TASK-001.json"
        item_path.write_bytes(render_task_item_contract(item))
        index = copy.deepcopy(self.fixture.index)
        index["tasks"][0]["canonical_sha256"] = "0" * 64
        (self.root / self.fixture.index_path).write_text(json.dumps(index))
        prepared = self.prepare_request()
        self.assertIn(self.fixture.index_path, prepared["preview"]["changed_paths"])
        result = self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        self.assertEqual(result["status"], "repaired")
        self.assertTrue(load_task_collection(self.root, str(self.root), self.fixture.index_path)["task_count"])

    def test_missing_item_is_restored_from_explicit_candidate(self):
        item_path = (self.root / self.fixture.index_path).parent / "tasks" / "TASK-001.json"
        item_path.unlink()
        prepared = self.prepare_request()
        self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        self.assertEqual(item_path.read_bytes(), self.fixture.item_raw)

    def test_missing_formal_index_is_restored_from_explicit_candidate(self):
        index_path = self.root / self.fixture.index_path
        index_path.unlink()
        prepared = self.prepare_request()
        self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        self.assertEqual(index_path.read_bytes(), self.fixture.index_raw)

    def test_orphan_requires_explicit_removal_decision_and_is_not_adopted(self):
        orphan = (self.root / self.fixture.index_path).parent / "tasks" / "TASK-999.json"
        orphan.write_bytes(self.fixture.item_raw)
        with self.assertRaises(WorkError) as caught:
            self.prepare_request()
        self.assertEqual(caught.exception.code, "task_repair_orphan_decision")
        prepared = self.prepare_request(decisions=[{"location": "/orphans/TASK-999.json", "decision": "Remove reviewed orphan."}])
        self.assertNotIn("TASK-999", prepared["request"]["task_items"])

    def test_binding_only_repair_preserves_task_status(self):
        execution_path = self.root / self.artifacts["execution"] / "index.json"
        execution = json.loads(execution_path.read_bytes())
        execution["task_collection_sha256"] = "0" * 64
        execution_path.write_bytes(render_execution_index(execution))
        prepared = self.prepare_request()
        self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        self.assertEqual(json.loads(execution_path.read_bytes())["tasks"][0]["status"], "pending")

    def test_interruption_recovers_identical_transaction(self):
        item_path = (self.root / self.fixture.index_path).parent / "tasks" / "TASK-001.json"
        item_path.unlink()
        prepared = self.prepare_request()
        real = spec_transactions.publish_journal
        with patch.object(spec_transactions, "publish_journal", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        result = self.run_repair(prepared, "recover", prepared["preview"]["approved_sha256"])
        self.assertEqual(result["status"], "recovered")
        repeated = self.run_repair(prepared, "recover", prepared["preview"]["approved_sha256"])
        self.assertEqual(repeated["status"], "already_completed")

    def test_conflicting_candidate_is_rejected(self):
        candidate = copy.deepcopy(self.fixture.item)
        candidate["id"] = "TASK-999"
        request = {
            "schema": "work-task-repair-prepare-request/v2", "stage": "complete",
            "requirement_id": "example", "artifacts": self.artifacts,
            "decisions": [{"location": "/", "decision": "Use candidate."}],
            "task_index": copy.deepcopy(self.fixture.index), "task_items": {"TASK-001": candidate},
        }
        with self.assertRaises(WorkError):
            prepare_task_repair(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root))


if __name__ == "__main__":
    unittest.main()
