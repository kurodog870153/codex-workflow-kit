from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.models.execution import ExecutionDeviationContract
from worklib.orchestration.task import (
    preview_specification_reconciliation, publish_specification_reconciliation,
)


class SpecificationReconciliationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.attempt_path = "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json"
        path = self.root / self.attempt_path
        path.parent.mkdir(parents=True)
        path.write_bytes(b"attempt\n")
        self.attempt = {"status": "completed", "execution_deviations": [
            copy.deepcopy(ExecutionDeviationContract.contract_example),
        ]}
        self.migration = {"schema": "work-spec-migration-preview-request/v1", "sources": [
            {"path": "source.json", "raw_sha256": "0" * 64}], "candidates": [
            {"path": "candidate.json", "kind": "plan", "content": {}}], "semantic_decisions": []}
        self.request = {"schema": "work-spec-reconciliation-preview-request/v1",
                        "attempt_path": self.attempt_path, "choice": "all",
                        "deviation_ids": [], "migration": self.migration}
        self.migration_preview = {"schema": "work-spec-migration-preview/v1", "status": "ready",
                                  "documents": [], "diffs": [], "validator_results": [],
                                  "relationship_results": [], "unresolved_items": [],
                                  "fingerprint": "a" * 64, "writable_ready": True}

    def preview(self):
        with patch("worklib.business_services.specification.reconciliation.render_attempt_json_contract", return_value=self.attempt), \
             patch("worklib.business_services.specification.reconciliation.preview_specification_migration", return_value=self.migration_preview):
            return preview_specification_reconciliation(json.dumps(self.request).encode(), project_root=self.root,
                                                        user_config_root=str(self.root), skill_roots=[])

    def test_all_and_selective_choices_bind_attempt_and_deviations(self):
        result = self.preview()
        self.assertEqual(result["selected_deviation_ids"], ["DEVIATION-001"])
        self.assertEqual(result["deviation_classifications"], {"DEVIATION-001": "task_only"})
        self.assertEqual(result["ledger"]["entries"][0]["outcome"], "incorporated")
        self.request["choice"] = "selective"
        self.request["deviation_ids"] = ["DEVIATION-001"]
        self.assertEqual(self.preview()["selected_deviation_ids"], ["DEVIATION-001"])

    def test_retain_only_is_read_only_and_needs_no_candidates(self):
        self.request.update(choice="retain_only", deviation_ids=[], migration=None)
        with patch("worklib.business_services.specification.reconciliation.render_attempt_json_contract", return_value=self.attempt), \
             patch("worklib.business_services.specification.reconciliation.preview_specification_migration") as migration:
            result = preview_specification_reconciliation(json.dumps(self.request).encode(), project_root=self.root,
                                                          user_config_root=str(self.root), skill_roots=[])
        self.assertEqual(result["status"], "ready")
        self.assertTrue(result["publication_required"])
        self.assertTrue(result["publication_ready"])
        self.assertEqual(result["deviation_classifications"], {"DEVIATION-001": "retain_only"})
        self.assertEqual(result["ledger"]["entries"][0]["outcome"], "retained")
        migration.assert_not_called()

    def test_retain_only_publishes_ledger_without_changing_attempt(self):
        self.request.update(choice="retain_only", deviation_ids=[], migration=None)
        preview = self.preview()
        publication = {
            "schema": "work-spec-migration-publication/v1",
            "status": "updated",
            "fingerprint": preview["fingerprint"],
            "transaction_approval_sha256": "b" * 64,
            "journal": "journal",
            "completion_marker": "journal.done",
            "documents": [preview["ledger_path"]],
            "publication_status": "published",
            "validator_results": [],
            "relationship_results": [],
        }
        attempt_before = (self.root / self.attempt_path).read_bytes()
        with patch(
            "worklib.business_services.specification.reconciliation.preview_specification_reconciliation",
            return_value=preview,
        ), patch(
            "worklib.business_services.specification.reconciliation._publish_ledger_only",
            return_value=publication,
        ) as publish:
            result = publish_specification_reconciliation(
                json.dumps(self.request).encode(),
                approved_sha256=preview["fingerprint"],
                project_root=self.root,
                user_config_root=str(self.root),
            )
        publish.assert_called_once()
        self.assertEqual((self.root / self.attempt_path).read_bytes(), attempt_before)
        self.assertEqual(result["retained_deviation_ids"], ["DEVIATION-001"])

    def test_apply_revalidates_fingerprint_and_delegates_to_migration_transaction(self):
        preview = self.preview()
        publication = {"schema": "work-spec-migration-publication/v1", "status": "updated",
                       "fingerprint": "a" * 64, "transaction_approval_sha256": "b" * 64,
                       "journal": "journal", "completion_marker": "journal.done", "documents": [],
                       "publication_status": "published", "validator_results": [], "relationship_results": []}
        with patch("worklib.business_services.specification.reconciliation.preview_specification_reconciliation", return_value=preview), \
             patch("worklib.business_services.specification.reconciliation.publish_specification_migration", return_value=publication) as publish:
            result = publish_specification_reconciliation(json.dumps(self.request).encode(), approved_sha256=preview["fingerprint"],
                                                          project_root=self.root, user_config_root=str(self.root), skill_roots=[])
        self.assertEqual(result["status"], "updated")
        self.assertEqual(publish.call_args.kwargs["approved_sha256"], "a" * 64)
        self.assertIn(preview["ledger_path"], publish.call_args.kwargs["additional_candidates"])

    def test_existing_ledger_prevents_repeated_reconciliation(self):
        preview = self.preview()
        ledger_path = self.root / preview["ledger_path"]
        ledger_path.parent.mkdir(parents=True, exist_ok=True)
        ledger_path.write_text(json.dumps(preview["ledger"]) + "\n", encoding="utf-8")
        with self.assertRaises(Exception) as caught:
            self.preview()
        self.assertEqual(caught.exception.code, "reconciliation_nothing_pending")


if __name__ == "__main__":
    unittest.main()
