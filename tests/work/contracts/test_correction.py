from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.correction import (
    canonicalize_correction_contract,
    validate_correction_contract,
)
from worklib.foundation.errors import WorkError


class CorrectionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.correction = {
            "schema": "work-correction/v1",
            "correction_id": "ATTEMPT-001-CORRECTION-001",
            "created_at": "2026-09-01T10:05+08:00",
            "target_attempt_id": "ATTEMPT-001",
            "task_instructions_sha256": "b" * 64,
            "execute_instructions_sha256": "c" * 64,
            "field": "records[0].outcome",
            "correct_value": "passed",
            "reason": "Correct the recorded outcome.",
        }

    def test_uses_instruction_fingerprints(self) -> None:
        canonical = canonicalize_correction_contract(self.correction)

        self.assertEqual(canonical["task_instructions_sha256"], "b" * 64)
        self.assertNotIn("task_rules_sha256", canonical)
        self.assertNotIn("execute_rules_sha256", canonical)

    def test_rejects_legacy_rule_fingerprints(self) -> None:
        correction = copy.deepcopy(self.correction)
        correction["task_rules_sha256"] = correction.pop(
            "task_instructions_sha256"
        )
        correction["execute_rules_sha256"] = correction.pop(
            "execute_instructions_sha256"
        )

        with self.assertRaises(WorkError) as context:
            canonicalize_correction_contract(correction)

        self.assertEqual(context.exception.code, "correction_invalid_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["execute_instructions_sha256", "task_instructions_sha256"],
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["execute_rules_sha256", "task_rules_sha256"],
        )

    def test_accepts_v2_collection_fingerprints(self) -> None:
        correction = copy.deepcopy(self.correction)
        correction["schema"] = "work-correction/v2"
        correction.update(
            {
                "task_collection_sha256": "1" * 64,
                "task_index_sha256": "2" * 64,
                "task_item_sha256": "3" * 64,
            }
        )

        result = validate_correction_contract(correction)

        self.assertEqual(result["schema"], "work-correction-validation/v2")

    def test_rejects_v1_correction_with_v2_fingerprints(self) -> None:
        correction = copy.deepcopy(self.correction)
        correction["task_collection_sha256"] = "1" * 64

        with self.assertRaises(WorkError) as context:
            canonicalize_correction_contract(correction)

        self.assertEqual(context.exception.code, "correction_invalid_fields")


if __name__ == "__main__":
    unittest.main()
