from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.models.hierarchy import HierarchyContract
from worklib.models.instruction import InstructionCatalogContract


class HierarchyInstructionContractTests(unittest.TestCase):
    def test_hierarchy_canonical_order(self) -> None:
        model = HierarchyContract.model_validate(HierarchyContract.contract_example)
        self.assertEqual(list(model.to_canonical_dict()), list(HierarchyContract.canonical_order))

    def test_instruction_catalog_canonical_order(self) -> None:
        model = InstructionCatalogContract.model_validate(InstructionCatalogContract.contract_example)
        self.assertEqual(list(model.to_canonical_dict()), list(InstructionCatalogContract.canonical_order))


if __name__ == "__main__":
    unittest.main()
