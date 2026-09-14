from __future__ import annotations

import sys
import unittest
from pathlib import Path

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.instructions.draft_selection import resolve_draft_instruction_selection, validate_draft_instruction_selection


class DraftSelectionTests(unittest.TestCase):
    def test_stored_order_is_preserved_and_result_is_independent(self):
        selection = {"selected_paths": ["web/backend", "web/frontend"], "references": ["second", "first"]}
        result = resolve_draft_instruction_selection({"instruction_selection": selection})
        self.assertEqual(result, selection)
        result["references"].append("third")
        self.assertEqual(selection["references"], ["second", "first"])

    def test_legacy_selection_is_explicit_and_empty_is_distinct_from_missing(self):
        with self.assertRaises(WorkError) as context:
            resolve_draft_instruction_selection({})
        self.assertEqual(context.exception.code, "draft_selection_required")
        self.assertEqual(resolve_draft_instruction_selection({}, selected_paths=[]), {"selected_paths": [], "references": []})

    def test_explicit_selection_cannot_replace_stored_choice(self):
        entry = {"instruction_selection": {"selected_paths": [], "references": ["saved"]}}
        self.assertEqual(resolve_draft_instruction_selection(entry, selected_paths=[], reference_names=["saved"]), entry["instruction_selection"])
        with self.assertRaises(WorkError) as context:
            resolve_draft_instruction_selection(entry, selected_paths=[])
        self.assertEqual(context.exception.code, "draft_selection_mismatch")

    def test_references_alone_are_not_a_complete_selection(self):
        with self.assertRaises(WorkError) as context:
            resolve_draft_instruction_selection({}, reference_names=[])
        self.assertEqual(context.exception.code, "draft_selection_incomplete")

    def test_malformed_selections_are_rejected(self):
        for value in (None, {}, {"selected_paths": [], "references": [], "extra": True},
                      {"selected_paths": ["web", "web"], "references": []},
                      {"selected_paths": [], "references": [" "]},
                      {"selected_paths": [], "references": [[]]},
                      {"selected_paths": "web", "references": []}):
            with self.subTest(value=value), self.assertRaises(WorkError):
                validate_draft_instruction_selection(value)


if __name__ == "__main__":
    unittest.main()
