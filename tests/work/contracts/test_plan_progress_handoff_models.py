from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.models.handoff import HandoffContract, HandoffSourceValidationContract, HandoffValidationContract
from worklib.models.handoff import (
    HandoffContract as ModelHandoffContract,
    HandoffSourceValidationContract as ModelHandoffSourceValidationContract,
    HandoffValidationContract as ModelHandoffValidationContract,
)
from worklib.models.plan import PlanContract, PlanCreateContract, PlanPrepareContract, PlanValidationContract
from worklib.models.progress import DiscussionProgressContract as LegacyDiscussionProgressContract
from worklib.models.progress import ProgressPrepareContract as LegacyProgressPrepareContract
from worklib.models.progress import ProgressPreviewContract as LegacyProgressPreviewContract
from worklib.models.progress import ProgressReadContract as LegacyProgressReadContract
from worklib.models.progress import ProgressSaveContract as LegacyProgressSaveContract
from worklib.models.progress import ProgressSaveRequestContract as LegacyProgressSaveRequestContract
from worklib.models.progress import (
    DiscussionProgressContract, ProgressPrepareContract, ProgressPreviewContract,
    ProgressReadContract, ProgressSaveContract, ProgressSaveRequestContract,
)


class PlanProgressHandoffModelTests(unittest.TestCase):
    def test_legacy_handoff_exports_preserve_class_identity(self) -> None:
        self.assertIs(HandoffContract, ModelHandoffContract)
        self.assertIs(HandoffValidationContract, ModelHandoffValidationContract)
        self.assertIs(
            HandoffSourceValidationContract,
            ModelHandoffSourceValidationContract,
        )

    def test_legacy_progress_exports_preserve_class_identity(self) -> None:
        pairs = (
            (LegacyDiscussionProgressContract, DiscussionProgressContract),
            (LegacyProgressPrepareContract, ProgressPrepareContract),
            (LegacyProgressPreviewContract, ProgressPreviewContract),
            (LegacyProgressReadContract, ProgressReadContract),
            (LegacyProgressSaveContract, ProgressSaveContract),
            (LegacyProgressSaveRequestContract, ProgressSaveRequestContract),
        )
        for legacy, current in pairs:
            self.assertIs(legacy, current)

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
