from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from pydantic import ValidationError

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.contracts.specification_migration_models import (
    SpecificationMigrationPreviewContract, SpecificationMigrationPreviewRequestContract,
    SpecificationMigrationPublicationContract,
)


class SpecificationMigrationContractTests(unittest.TestCase):
    def test_examples_validate(self):
        for contract in (SpecificationMigrationPreviewRequestContract, SpecificationMigrationPreviewContract,
                         SpecificationMigrationPublicationContract):
            self.assertEqual(contract.model_validate(contract.contract_example).to_canonical_dict(), contract.contract_example)

    def test_task_id_is_limited_to_task_items(self):
        value = copy.deepcopy(SpecificationMigrationPreviewRequestContract.contract_example)
        value["candidates"][0]["task_id"] = "TASK-001"
        with self.assertRaises(ValidationError):
            SpecificationMigrationPreviewRequestContract.model_validate(value)

    def test_readiness_cannot_ignore_failed_checks_or_unresolved_items(self):
        for change in (
            {"validator_results": [{"name": "plan", "status": "failed", "code": "invalid", "message": "invalid"}]},
            {"unresolved_items": ["DECISION-001"]},
        ):
            value = copy.deepcopy(SpecificationMigrationPreviewContract.contract_example)
            value.update(change)
            with self.assertRaises(ValidationError):
                SpecificationMigrationPreviewContract.model_validate(value)


if __name__ == "__main__":
    unittest.main()
