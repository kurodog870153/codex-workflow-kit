from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.services.specification.transaction import derived_transaction_id, encode_snapshot, render_spec_transaction, transaction_approval_sha256, validate_spec_transaction
from worklib.models.common.errors import WorkError


class SpecTransactionContractTests(unittest.TestCase):
    def contract(self):
        metadata = {"request": {"schema": "test/v1"}, "artifacts": {}, "affected_task_ids": ["TASK-001"], "history_sha256": {}, "source_sha256": {}, "candidate_sha256": {}}
        files = [
            {"phase": 20, "path": "tasks/TASK-001.json", "operation": "replace", "before": encode_snapshot(b"old"), "after": encode_snapshot(b"new")},
            {"phase": 20, "path": "tasks/TASK-002.json", "operation": "add", "after": encode_snapshot(b"added")},
        ]
        approval = transaction_approval_sha256(files, metadata)
        return {"schema": "work-spec-transaction/v1", "transaction_id": derived_transaction_id("UPDATE", approval), "approval_sha256": approval, "state": "prepared", "published_count": 0, "metadata": metadata, "files": files}

    def test_id_is_stable_and_rejects_caller_selected_identity(self):
        first = self.contract()
        self.assertEqual(first["transaction_id"], self.contract()["transaction_id"])
        different = copy.deepcopy(first)
        different["metadata"]["request"] = {"schema": "different/v1"}
        different["approval_sha256"] = transaction_approval_sha256(different["files"], different["metadata"])
        different["transaction_id"] = derived_transaction_id("UPDATE", different["approval_sha256"])
        self.assertNotEqual(first["transaction_id"], different["transaction_id"])
        first["transaction_id"] = "SPEC-UPDATE-002"
        with self.assertRaises(WorkError):
            render_spec_transaction(first)

    def test_shared_journal_keeps_other_workflow_identifiers(self):
        for identity in ("TASK-REPAIR-ABCDEF012345", "SOURCE-REFRESH-ABCDEF012345",
                         "INSTRUCTION-MIGRATION-ABCDEF012345"):
            with self.subTest(identity=identity):
                transaction = self.contract()
                transaction["transaction_id"] = identity
                self.assertEqual(validate_spec_transaction(render_spec_transaction(transaction), source="test")["transaction_id"], identity)

    def test_round_trip(self):
        raw = render_spec_transaction(self.contract())
        self.assertEqual(validate_spec_transaction(raw, source="test")["published_count"], 0)

    def test_rejects_snapshot_approval_order_and_progress_mismatches(self):
        cases = []
        broken = copy.deepcopy(self.contract()); broken["files"][0]["before"]["base64"] = "bmV3"; cases.append(broken)
        broken = copy.deepcopy(self.contract()); broken["approval_sha256"] = "0" * 64; cases.append(broken)
        broken = copy.deepcopy(self.contract()); broken["files"].reverse(); broken["approval_sha256"] = transaction_approval_sha256(broken["files"], broken["metadata"]); cases.append(broken)
        broken = copy.deepcopy(self.contract()); broken["published_count"] = 1; cases.append(broken)
        for contract in cases:
            with self.subTest(contract=contract), self.assertRaises(WorkError):
                render_spec_transaction(contract)

    def test_rejects_retired_schema(self):
        contract = self.contract()
        contract["schema"] = "work-spec-transaction/v2"

        with self.assertRaises(WorkError):
            render_spec_transaction(contract)
