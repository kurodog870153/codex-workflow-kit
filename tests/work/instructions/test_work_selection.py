from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
sys.path.insert(0, str(SKILL_ROOT / "scripts"))

from worklib.foundation.errors import WorkError
from worklib.instructions.work_selection import (
    build_work_instruction_selection,
    validate_work_instruction_selection,
)


class WorkInstructionSelectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.selection = build_work_instruction_selection(
            skill_root=SKILL_ROOT, mode="plan", selected_paths=[]
        )

    def test_valid_selection_has_explicit_mode_topology(self) -> None:
        loaded = validate_work_instruction_selection(
            self.selection,
            skill_root=SKILL_ROOT,
            mode="plan",
            selected_paths=[],
        )
        self.assertEqual(loaded.instructions_sha256, self.selection["instructions_sha256"])
        self.assertEqual(self.selection["selected_paths"], [])
        self.assertEqual(self.selection["resolved_paths"], ["general"])

    def test_selected_paths_must_match_confirmed_hierarchy(self) -> None:
        selection = copy.deepcopy(self.selection)
        selection["selected_paths"] = ["web"]
        with self.assertRaises(WorkError) as context:
            validate_work_instruction_selection(
                selection,
                skill_root=SKILL_ROOT,
                mode="plan",
                selected_paths=[],
            )
        self.assertEqual(
            context.exception.code,
            "work_instruction_selection_selected_paths_mismatch",
        )

    def test_stale_fingerprint_is_rejected(self) -> None:
        selection = copy.deepcopy(self.selection)
        selection["instructions_sha256"] = "0" * 64
        with self.assertRaises(WorkError) as context:
            validate_work_instruction_selection(
                selection,
                skill_root=SKILL_ROOT,
                mode="plan",
                selected_paths=[],
            )
        self.assertEqual(context.exception.code, "work_instructions_fingerprint_mismatch")


if __name__ == "__main__":
    unittest.main()
