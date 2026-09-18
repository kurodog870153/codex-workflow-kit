from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.models.skill import (
    SkillBundleContract, SkillCatalogContract, SkillSelectionContract,
    SkillSelectionValidationContract, SkillSnapshotContract,
)


class SkillContractTests(unittest.TestCase):
    def test_examples_validate_and_use_canonical_order(self) -> None:
        for contract in (
            SkillBundleContract, SkillCatalogContract, SkillSelectionContract,
            SkillSelectionValidationContract, SkillSnapshotContract,
        ):
            with self.subTest(contract=contract.contract_id):
                model = contract.model_validate(contract.contract_example)
                self.assertEqual(list(model.to_canonical_dict()), list(contract.canonical_order))


if __name__ == "__main__":
    unittest.main()
