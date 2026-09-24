from __future__ import annotations

import copy
import hashlib
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts.test_task_collection import TaskCollectionTests
from worklib.business_services.task import load_task_collection
from worklib.orchestration.task import prepare_task_repair, repair_task
from worklib.services.attempt.validation import build_initial_execution_index, render_execution_index
from worklib.business_services.task.item import render_task_item_contract, validate_task_item_contract
from worklib.models.common.errors import WorkError
from worklib.services.task.repair_fingerprint import (
    fingerprint_task_repair_contents,
    fingerprint_task_repair_evidence,
    task_repair_transaction_id,
)
from worklib.services.task.draft.validation import build_semantic_task_patch


class TaskRepairFingerprintTests(unittest.TestCase):
    def test_fingerprints_repair_identity_and_complete_evidence(self):
        request_raw = b'{"schema":"work-task-repair-request/v1"}\n'
        contents = {"tasks/index.json": b"index\n", "tasks/tasks/TASK-001.json": b"item\n"}

        self.assertEqual(
            task_repair_transaction_id(request_raw),
            "TASK-REPAIR-" + hashlib.sha256(request_raw).hexdigest()[:12].upper(),
        )
        self.assertEqual(
            fingerprint_task_repair_contents(contents),
            {path: hashlib.sha256(raw).hexdigest() for path, raw in contents.items()},
        )
        self.assertEqual(
            fingerprint_task_repair_evidence(
                contents, ["tasks/index.json", "execution/index.json"]
            ),
            {
                "tasks/index.json": hashlib.sha256(b"index\n").hexdigest(),
                "execution/index.json": None,
            },
        )

    def test_nested_builder_allocates_ids_and_resolves_local_keys(self):
        current = {"commands": [{"id": "CMD-001", "mode": "argv", "argv": ["tool", "old"]}],
                   "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "user", "criteria": "Old"}]}
        built = build_semantic_task_patch(current, {
            "commands": [{"key": "old", "existing_position": 1, "mode": "argv", "argv": ["tool", "old"]},
                         {"key": "rerun", "mode": "argv", "argv": ["tool", "new"]}],
            "validations": [{"key": "verify", "kind": "automated", "command_keys": ["rerun"],
                             "pass_condition": "Exit zero", "acceptance_positions": [1]}],
        }, acceptance_ids=["ACCEPTANCE-001"])
        self.assertEqual(built["commands"][1]["id"], "CMD-002")
        self.assertEqual(built["validations"][0]["id"], "VAL-002")
        self.assertEqual(built["validations"][0]["command_ids"], ["CMD-002"])
        self.assertEqual(built["validations"][0]["acceptance_ids"], ["ACCEPTANCE-001"])


class TaskRepairTests(unittest.TestCase):
    def setUp(self):
        fixture = TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.root = fixture.root
        self.artifacts = fixture.index["artifacts"]
        validation = load_task_collection(self.root, str(self.root), fixture.index_path)
        execution = build_initial_execution_index(validation["collection_contract"], validation)
        execution_path = self.root / self.artifacts["execution"] / "index.json"
        execution_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.write_bytes(render_execution_index(execution))

    def prepare_request(self, *, decisions=None, edits=None, missing_task=None):
        request = {
            "schema": "work-task-repair-prepare-request/v1", "stage": "complete",
            "requirement_id": "example",
            "decisions": decisions or [{"location": "/", "decision": "Use the reviewed candidate."}],
        }
        if edits is not None:
            request["edits"] = edits
        if missing_task is not None:
            request["missing_task"] = missing_task
        return prepare_task_repair(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root))

    def semantic_missing_task(self):
        return {"task_position": 1, "title": self.fixture.item["title"], "goal": self.fixture.item["goal"],
                "skill_id": None, "selected_paths": [], "references": ["task.general.task-records"],
                "dependency_positions": [], "candidate": {
                    "files": [{"key": "source", "action": "modify", "path": "src.txt"}],
                    "commands": [{"key": "check", "mode": "argv", "argv": ["python", "--version"]}],
                    "validations": [{"key": "passes", "kind": "automated", "command_keys": ["check"],
                                     "pass_condition": "Exit code is zero.", "acceptance_positions": [1]}],
                    "steps": [{"key": "modify", "action": "Modify the source.", "references": [{"kind": "files", "key": "source"}]},
                              {"key": "validate", "action": "Run validation.", "references": [{"kind": "commands", "key": "check"}, {"kind": "validations", "key": "passes"}]}],
                }}

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
        prepared = self.prepare_request(edits=[{"task_id": "TASK-001", "field": "title", "after": self.fixture.item["title"]}])
        self.assertIn(self.fixture.index_path, prepared["preview"]["changed_paths"])
        result = self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        self.assertEqual(result["status"], "repaired")
        self.assertTrue(load_task_collection(self.root, str(self.root), self.fixture.index_path)["task_count"])

    def test_nested_file_repair_uses_semantic_position(self):
        item = copy.deepcopy(self.fixture.item)
        item["files"][0]["path"] = "wrong.txt"
        item_path = (self.root / self.fixture.index_path).parent / "tasks" / "TASK-001.json"
        item_path.write_bytes(render_task_item_contract(item))
        prepared = self.prepare_request(edits=[{"task_id": "TASK-001", "field": "files",
            "semantic_after": [{"key": "source", "existing_position": 1, "action": "modify", "path": "src.txt"}]}])
        repaired = prepared["request"]["task_items"]["TASK-001"]
        self.assertEqual(repaired["files"], self.fixture.item["files"])
        self.assertEqual(repaired["steps"], self.fixture.item["steps"])

    def test_missing_item_is_restored_from_explicit_candidate(self):
        item_path = (self.root / self.fixture.index_path).parent / "tasks" / "TASK-001.json"
        item_path.unlink()
        prepared = self.prepare_request(missing_task=self.semantic_missing_task())
        self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        self.assertEqual(item_path.read_bytes(), self.fixture.item_raw)

    def test_missing_formal_index_requires_reviewed_source(self):
        index_path = self.root / self.fixture.index_path
        index_path.unlink()
        with self.assertRaises(WorkError) as caught:
            self.prepare_request()
        self.assertEqual(caught.exception.code, "task_repair_ambiguous_source")

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
        prepared = self.prepare_request(missing_task=self.semantic_missing_task())
        with patch(
            "worklib.business_services.task.repair.publish_journal",
            side_effect=OSError("interrupted"),
        ):
            with self.assertRaises(WorkError):
                self.run_repair(prepared, "apply", prepared["preview"]["approved_sha256"])
        result = self.run_repair(prepared, "recover", prepared["preview"]["approved_sha256"])
        self.assertEqual(result["status"], "recovered")
        repeated = self.run_repair(prepared, "recover", prepared["preview"]["approved_sha256"])
        self.assertEqual(repeated["status"], "already_completed")


class TaskRepairPreparationTests(unittest.TestCase):
    def setUp(self):
        fixture = TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.root = fixture.root
        self.artifacts = fixture.index["artifacts"]

    def test_conflicting_candidate_is_rejected(self):
        candidate = copy.deepcopy(self.fixture.item)
        candidate["id"] = "TASK-999"
        request = {
            "schema": "work-task-repair-prepare-request/v1",
            "stage": "complete",
            "requirement_id": "example",
            "artifacts": self.artifacts,
            "decisions": [{"location": "/", "decision": "Use candidate."}],
            "task": candidate,
        }
        with self.assertRaises(WorkError):
            prepare_task_repair(
                json.dumps(request).encode(),
                project_root=self.root,
                user_config_root=str(self.root),
            )


if __name__ == "__main__":
    unittest.main()
