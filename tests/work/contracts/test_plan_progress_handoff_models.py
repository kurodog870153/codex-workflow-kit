from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.contracts.handoff import HandoffContract, HandoffSourceValidationContract, HandoffValidationContract
from worklib.contracts.plan import PlanContract, PlanCreateContract, PlanPrepareContract, PlanValidationContract
from worklib.contracts.progress import DiscussionProgressContract, ProgressPrepareContract, ProgressPreviewContract, ProgressReadContract, ProgressSaveContract, ProgressSaveRequestContract


class PlanProgressHandoffModelTests(unittest.TestCase):
    def test_examples_validate_and_use_canonical_order(self) -> None:
        contracts = (
            PlanContract, PlanCreateContract, PlanPrepareContract, PlanValidationContract,
            DiscussionProgressContract, ProgressPrepareContract, ProgressPreviewContract,
            ProgressReadContract, ProgressSaveContract, ProgressSaveRequestContract,
            HandoffContract, HandoffSourceValidationContract, HandoffValidationContract,
        )
        for contract in contracts:
            with self.subTest(contract=contract.contract_id):
                model = contract.model_validate(contract.contract_example)
                rendered = model.to_canonical_dict()
                expected = [
                    field for field in contract.canonical_order if field in rendered
                ]
                self.assertEqual(list(rendered), expected)


if __name__ == "__main__":
    unittest.main()
