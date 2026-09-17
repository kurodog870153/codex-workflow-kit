from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.services.task_repair import prepare_task_repair, repair_task
from worklib.contracts.registry import registry
from worklib.contracts.task_repair_models import (
    TaskRepairPrepareContract, TaskRepairPrepareRequestContract,
    TaskRepairContract, TaskRepairRequestContract,
)
from worklib.foundation.errors import WorkError


class TaskRepairContractTests(unittest.TestCase):
    def test_registered_examples_round_trip(self):
        contracts = (TaskRepairPrepareRequestContract, TaskRepairRequestContract,
                     TaskRepairContract, TaskRepairPrepareContract)
        for contract in contracts:
            with self.subTest(contract=contract.contract_id):
                example = registry.describe(contract.contract_id).example
                model = contract.model_validate(example)
                self.assertEqual(model.to_canonical_dict(), example)
                self.assertEqual(contract.parse_json_bytes(model.render_canonical_json(), source="test").to_canonical_dict(), example)

    def test_prepare_rejects_invalid_structure_before_source_reads(self):
        cases = [[], {**TaskRepairPrepareRequestContract.contract_example, "schema": "work-task-repair-prepare-request/v2"},
                 {**TaskRepairPrepareRequestContract.contract_example, "decisions": []},
                 {**TaskRepairPrepareRequestContract.contract_example, "task_items": {}},
                 {**TaskRepairPrepareRequestContract.contract_example, "unexpected": True}]
        for request in cases:
            with self.subTest(request=request), patch("worklib.services.task_repair._sources") as sources:
                with self.assertRaises(WorkError):
                    prepare_task_repair(json.dumps(request).encode(), project_root=Path("."), user_config_root=".")
                sources.assert_not_called()

    def test_repair_rejects_invalid_expected_before_source_reads(self):
        for invalid in ("A" * 64, "0" * 63, 1, False):
            request = copy.deepcopy(TaskRepairRequestContract.contract_example)
            request["expected"]["index.json"] = invalid
            with self.subTest(invalid=invalid), patch("worklib.services.task_repair.diagnose_task_collection") as diagnose:
                with self.assertRaises(WorkError) as caught:
                    repair_task(json.dumps(request).encode(), project_root=Path("."), user_config_root=".")
                self.assertEqual(caught.exception.code, "task_repair_expected")
                diagnose.assert_not_called()

    def test_expected_explicit_null_is_preserved(self):
        request = copy.deepcopy(TaskRepairRequestContract.contract_example)
        request["expected"]["missing.json"] = None
        model = TaskRepairRequestContract.model_validate(request)
        self.assertEqual(model.to_canonical_dict(), request)

    def test_response_preserves_preview_and_publication_shapes(self):
        preview = copy.deepcopy(TaskRepairContract.contract_example)
        self.assertNotIn("publication_status", TaskRepairContract.model_validate(preview).to_canonical_dict())
        preview.update(status="repaired", publication_status="published")
        self.assertEqual(TaskRepairContract.model_validate(preview).publication_status, "published")
