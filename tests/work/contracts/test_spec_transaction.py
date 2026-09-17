from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.spec_transaction import encode_snapshot, render_spec_transaction, transaction_approval_sha256, validate_spec_transaction
from worklib.foundation.errors import WorkError


class SpecTransactionContractTests(unittest.TestCase):
    def contract(self):
        metadata = {"request": {"schema": "test/v1"}, "artifacts": {}, "affected_task_ids": ["TASK-001"], "history_sha256": {}, "source_sha256": {}, "candidate_sha256": {}}
        files = [
            {"phase": 20, "path": "tasks/TASK-001.json", "operation": "replace", "before": encode_snapshot(b"old"), "after": encode_snapshot(b"new")},
            {"phase": 20, "path": "tasks/TASK-002.json", "operation": "add", "after": encode_snapshot(b"added")},
        ]
        return {"schema": "work-spec-transaction/v2", "transaction_id": "SPEC-UPDATE-002", "approval_sha256": transaction_approval_sha256(files, metadata), "state": "prepared", "published_count": 0, "metadata": metadata, "files": files}

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
