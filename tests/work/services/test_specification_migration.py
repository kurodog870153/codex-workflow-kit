from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.foundation.fingerprint import raw_sha256
from worklib.services.specification_migration import preview_specification_migration, publish_specification_migration


class SpecificationMigrationPreviewTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        paths = {
            "plan": "outputs/work/plans/example.json",
            "task_index": "outputs/work/tasks/example/index.json",
            "task_item": "outputs/work/tasks/example/tasks/TASK-001.json",
            "execution_index": "outputs/work/executions/example/index.json",
        }
        self.paths = paths
        self.sources = []
        for path in paths.values():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b'{"schema":"same-id-but-incompatible"}\n')
            self.sources.append({"path": path, "raw_sha256": raw_sha256(target.read_bytes())})
        artifacts = {"plan": paths["plan"], "task": paths["task_index"], "execution": "outputs/work/executions/example"}
        self.candidates = [
            {"path": paths["plan"], "kind": "plan", "content": {"schema": "work-plan/v1", "artifacts": artifacts}},
            {"path": paths["task_index"], "kind": "task_index", "content": {"schema": "work-task-index/v1"}},
            {"path": paths["task_item"], "kind": "task_item", "task_id": "TASK-001", "content": {"schema": "work-task-item/v1"}},
            {"path": paths["execution_index"], "kind": "execution_index", "content": {"schema": "work-execution-index/v1", "tasks": [{"id": "TASK-001", "task_item_sha256": "3" * 64}], "requirement_id": "example", "task_spec_id": "TASK-SPEC-001", "task_collection_sha256": "1" * 64, "task_index_sha256": "2" * 64}},
        ]
        self.request = {"schema": "work-spec-migration-preview-request/v1", "sources": self.sources,
                        "candidates": self.candidates, "semantic_decisions": []}
        self.collection = {"requirement_id": "example", "spec_id": "TASK-SPEC-001", "task_ids": ["TASK-001"],
                           "task_collection_sha256": "1" * 64, "task_index_sha256": "2" * 64,
                           "task_item_sha256": {"TASK-001": "3" * 64}}

    def preview(self):
        with patch("worklib.services.specification_migration.validate_plan_contract", return_value={}), \
             patch("worklib.services.specification_migration.validate_task_collection_contract", return_value=self.collection), \
             patch("worklib.services.specification_migration.validate_execution_index", return_value={}):
            return preview_specification_migration(json.dumps(self.request).encode(), project_root=self.root,
                                                   user_config_root=str(self.root), skill_roots=[])

    def test_invalid_legacy_sources_are_evidence_not_validator_inputs(self):
        before = {path: (self.root / path).read_bytes() for path in self.paths.values()}
        result = self.preview()
        self.assertEqual(result["status"], "ready")
        self.assertTrue(result["writable_ready"])
        self.assertEqual(before, {path: (self.root / path).read_bytes() for path in self.paths.values()})

    def test_unresolved_semantics_block_and_decisions_change_fingerprint(self):
        self.request["semantic_decisions"] = [{"id": "DECISION-001", "question": "Which meaning?", "resolution": None}]
        blocked = self.preview()
        self.assertEqual((blocked["status"], blocked["unresolved_items"]), ("blocked", ["DECISION-001"]))
        self.request["semantic_decisions"][0]["resolution"] = "Use the confirmed v1 meaning."
        ready = self.preview()
        self.assertEqual(ready["status"], "ready")
        self.assertNotEqual(blocked["fingerprint"], ready["fingerprint"])

    def test_relationship_failure_blocks_writable_state(self):
        self.candidates[0]["content"]["artifacts"]["task"] = "wrong/index.json"
        result = self.preview()
        self.assertEqual(result["status"], "blocked")
        self.assertFalse(result["writable_ready"])

    def test_apply_publishes_and_recovery_reuses_identical_journal(self):
        preview = self.preview()
        with patch("worklib.services.specification_migration.validate_plan_contract", return_value={}), \
             patch("worklib.services.specification_migration.validate_task_collection_contract", return_value=self.collection), \
             patch("worklib.services.specification_migration.validate_execution_index", return_value={}):
            result = publish_specification_migration(
                json.dumps(self.request).encode(), project_root=self.root, user_config_root=str(self.root),
                skill_roots=[], operation="apply", approved_sha256=preview["fingerprint"],
            )
            recovered = publish_specification_migration(
                json.dumps(self.request).encode(), project_root=self.root, user_config_root=str(self.root),
                skill_roots=[], operation="recover", approved_sha256=preview["fingerprint"],
            )
        self.assertEqual(result["status"], "updated")
        self.assertEqual(recovered["status"], "recovered")
        self.assertEqual(recovered["publication_status"], "already_published")

    def test_source_drift_rejects_apply(self):
        preview = self.preview()
        (self.root / self.paths["plan"]).write_bytes(b"changed\n")
        with self.assertRaises(Exception) as caught:
            publish_specification_migration(
                json.dumps(self.request).encode(), project_root=self.root, user_config_root=str(self.root),
                skill_roots=[], operation="apply", approved_sha256=preview["fingerprint"],
            )
        self.assertEqual(caught.exception.code, "migration_source_changed")


if __name__ == "__main__":
    unittest.main()
