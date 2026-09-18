from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from pydantic import ValidationError

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.contracts.specification_reconciliation_models import (
    SpecificationReconciliationPreviewContract,
    SpecificationReconciliationPreviewRequestContract,
    SpecificationReconciliationPublicationContract,
)


class SpecificationReconciliationContractTests(unittest.TestCase):
    def test_examples_validate(self):
        for contract in (SpecificationReconciliationPreviewRequestContract,
                         SpecificationReconciliationPreviewContract,
                         SpecificationReconciliationPublicationContract):
            self.assertEqual(contract.model_validate(contract.contract_example).to_canonical_dict(), contract.contract_example)

    def test_retain_only_rejects_candidates(self):
        value = copy.deepcopy(SpecificationReconciliationPreviewRequestContract.contract_example)
        value["choice"] = "retain_only"
        with self.assertRaises(ValidationError):
            SpecificationReconciliationPreviewRequestContract.model_validate(value)


if __name__ == "__main__":
    unittest.main()
