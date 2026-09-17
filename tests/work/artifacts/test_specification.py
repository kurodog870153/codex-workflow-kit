from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts import test_task_collection
from worklib.services.specification import prepare_specification, update_specification, verify_specification
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
)
from worklib.foundation import spec_transactions
from worklib.foundation.errors import WorkError
from worklib.contracts.specification_models import SpecificationUpdateRequestContract
from worklib.services.task_collection import load_task_collection


class SpecificationCollectionUpdateTests(unittest.TestCase):
    def setUp(self):
        fixture = test_task_collection.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.root = fixture.root
        self.task_path = fixture.index_path
        self.plan_path = fixture.fixture.artifacts["plan"]
        validation = load_task_collection(self.root, str(self.root), self.task_path)
        self.execution = validation["collection_contract"]["artifacts"]["execution"]
        execution_path = self.root / self.execution
        execution_path.mkdir(parents=True, exist_ok=True)
        (execution_path / "index.json").write_bytes(
            render_execution_index(build_initial_execution_index(validation["collection_contract"], validation))
        )
        self.common = {"project_root": self.root, "user_config_root": str(self.root)}

    def request(self, edits):
        return {"schema": "work-spec-prepare-request/v1", "plan_path": self.plan_path,
                "reason": "Confirmed collection revision", "edits": edits}

    def prepare(self, edits):
        return prepare_specification(json.dumps(self.request(edits)).encode(), **self.common)

    def apply(self, prepared):
        request = json.dumps(prepared["request"]).encode()
        return update_specification(request, operation="apply",
                                    approved_sha256=prepared["preview"]["approved_sha256"], **self.common)

    def test_single_item_publish_preserves_other_item_and_verifies(self):
        first = json.loads((self.root / self.task_path).parent.joinpath("tasks/TASK-001.json").read_bytes())
        second = copy.deepcopy(first)
        second["id"] = "TASK-002"
        second["dependencies"] = ["TASK-001"]
        added = self.prepare([{"artifact": "task_item", "task_id": "TASK-002", "operation": "add",
                               "path": "/", "after": second}])
        self.apply(added)
        untouched = (self.root / self.task_path).parent.joinpath("tasks/TASK-002.json").read_bytes()
        current = json.loads((self.root / self.task_path).parent.joinpath("tasks/TASK-001.json").read_bytes())
        prepared = self.prepare([{"artifact": "task_item", "task_id": "TASK-001", "operation": "replace",
                                  "path": "/goal", "before": current["goal"], "after": current["goal"] + " confirmed"}])
        self.assertEqual(prepared["preview"]["changed_fields"], ["/task_items/TASK-001/goal"])
        result = self.apply(prepared)
        self.assertEqual((self.root / self.task_path).parent.joinpath("tasks/TASK-002.json").read_bytes(), untouched)
        verified = verify_specification(json.dumps(result["verification_request"]).encode(), **self.common)
        self.assertTrue(verified["verified"])

    def test_index_only_and_item_removal_candidates_validate(self):
        index = json.loads((self.root / self.task_path).read_bytes())
        prepared = self.prepare([{"artifact": "task_index", "operation": "replace", "path": "/summary",
                                  "before": index["summary"], "after": index["summary"] + " revised"}])
        self.assertEqual(prepared["preview"]["status"], "valid")
        first = json.loads((self.root / self.task_path).parent.joinpath("tasks/TASK-001.json").read_bytes())
        second = copy.deepcopy(first); second["id"] = "TASK-002"; second["dependencies"] = ["TASK-001"]
        added = self.prepare([{"artifact": "task_item", "task_id": "TASK-002", "operation": "add", "path": "/", "after": second}])
        self.apply(added)
        removal = self.prepare([{"artifact": "task_item", "task_id": "TASK-002", "operation": "remove", "path": "/", "before": second}])
        self.assertEqual(removal["preview"]["status"], "valid")
        result = self.apply(removal)
        self.assertFalse((self.root / self.task_path).parent.joinpath("tasks/TASK-002.json").exists())
        self.assertTrue(verify_specification(json.dumps(result["verification_request"]).encode(), **self.common)["verified"])

    def test_interrupted_publication_recovers_identical_request(self):
        current = json.loads((self.root / self.task_path).parent.joinpath("tasks/TASK-001.json").read_bytes())
        prepared = self.prepare([{"artifact": "task_item", "task_id": "TASK-001", "operation": "replace",
                                  "path": "/goal", "before": current["goal"], "after": current["goal"] + " recovered"}])
        request = json.dumps(prepared["request"]).encode()
        approval = prepared["preview"]["approved_sha256"]
        original = spec_transactions._replace_journal
        calls = 0
        def interrupt(*args, **kwargs):
            nonlocal calls
            calls += 1
            result = original(*args, **kwargs)
            if calls == 1:
                raise OSError("simulated interruption")
            return result
        with patch.object(spec_transactions, "_replace_journal", side_effect=interrupt), self.assertRaises(WorkError) as caught:
            update_specification(request, operation="apply", approved_sha256=approval, **self.common)
        self.assertEqual(caught.exception.code, "spec_update_interrupted")
        self.assertTrue(caught.exception.details["recovery_required"])
        recovered = update_specification(request, operation="recover", approved_sha256=approval, **self.common)
        self.assertEqual(recovered["status"], "recovered")
        self.assertTrue(verify_specification(json.dumps(recovered["verification_request"]).encode(), **self.common)["verified"])

    def test_prepared_typed_request_preserves_approval_on_transport(self):
        index = json.loads((self.root / self.task_path).read_bytes())
        prepared = self.prepare([{"artifact": "task_index", "operation": "replace", "path": "/summary",
                                  "before": index["summary"], "after": index["summary"] + " reviewed"}])
        model = SpecificationUpdateRequestContract.model_validate(prepared["request"])
        self.assertEqual(model.to_canonical_dict(), prepared["request"])
        validated = update_specification(model.render_canonical_json(), **self.common)
        self.assertEqual(validated["approved_sha256"], prepared["preview"]["approved_sha256"])
        stale = copy.deepcopy(prepared["request"])
        stale["expected"]["plan_sha256"] = "0" * 64
        with self.assertRaises(WorkError) as caught:
            update_specification(json.dumps(stale).encode(), **self.common)
        self.assertEqual(caught.exception.code, "spec_update_source_changed")


if __name__ == "__main__":
    unittest.main()
