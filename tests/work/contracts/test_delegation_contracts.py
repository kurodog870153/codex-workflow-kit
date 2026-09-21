from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.models.delegation import (
    DelegationEnvelopeContract, DelegationValidationContract,
)
from worklib.models.delegation import (
    DelegationEnvelopeContract as DelegationEnvelopeModel,
    DelegationValidationContract as DelegationValidationModel,
)


class DelegationContractTests(unittest.TestCase):
    def test_legacy_contract_imports_reexport_models(self) -> None:
        self.assertIs(DelegationEnvelopeContract, DelegationEnvelopeModel)
        self.assertIs(DelegationValidationContract, DelegationValidationModel)

    def test_examples_validate_and_use_canonical_order(self) -> None:
        for contract in (DelegationEnvelopeContract, DelegationValidationContract):
            with self.subTest(contract=contract.contract_id):
                model = contract.model_validate(contract.contract_example)
                self.assertEqual(list(model.to_canonical_dict()), list(contract.canonical_order))


if __name__ == "__main__":
    unittest.main()
