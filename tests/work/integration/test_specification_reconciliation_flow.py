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
from worklib.orchestration.task import preview_specification_reconciliation


class SpecificationReconciliationFlowTests(unittest.TestCase):
    def test_closed_attempt_selection_wraps_complete_migration_preview(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            attempt_path = "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json"
            target = root / attempt_path
            target.parent.mkdir(parents=True)
            target.write_bytes(b"immutable attempt\n")
            attempt = {"status": "completed", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
                       "task_spec_id": "TASK-SPEC-001",
                       "execution_deviations": [copy.deepcopy(ExecutionDeviationContract.contract_example)]}
            index_path = root / "outputs/work/executions/example/index.json"
            index_path.write_text(json.dumps({"schema": "work-execution-index/v1", "task_spec_id": "TASK-SPEC-001",
                                              "tasks": [{"id": "TASK-001", "latest_attempt": "ATTEMPT-001"}]}), encoding="utf-8")
            migration = {"schema": "work-spec-migration-preview-request/v1", "sources": [{"path": "old", "raw_sha256": "0" * 64}],
                         "candidates": [{"path": "new", "kind": "plan", "content": {}}], "semantic_decisions": []}
            request = {"schema": "work-spec-reconciliation-preview-request/v1", "attempt_path": attempt_path,
                       "choice": "all", "deviation_ids": [], "migration": migration}
            nested = {"schema": "work-spec-migration-preview/v1", "status": "ready", "documents": [], "diffs": [],
                      "validator_results": [], "relationship_results": [], "unresolved_items": [],
                      "fingerprint": "a" * 64, "writable_ready": True}
            with patch("worklib.business_services.specification.reconciliation.render_attempt_json_contract", return_value=attempt), \
                 patch("worklib.business_services.specification.reconciliation.preview_specification_migration", return_value=nested):
                result = preview_specification_reconciliation(json.dumps(request).encode(), project_root=root, user_config_root=str(root))
            self.assertEqual(result["selected_deviation_ids"], ["DEVIATION-001"])
            self.assertEqual(result["ledger"]["entries"][0]["outcome"], "incorporated")
            self.assertEqual(target.read_bytes(), b"immutable attempt\n")


if __name__ == "__main__":
    unittest.main()
