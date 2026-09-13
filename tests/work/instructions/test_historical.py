from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.foundation.errors import WorkError
from worklib.instructions.historical import stored_document_selection, stored_selection


def selection(name="task.general", fingerprint="a"):
    return {"selected_paths": [], "resolved_paths": ["general"], "references": [],
            "instructions_sha256": fingerprint * 64,
            "sources": [{"kind": "instruction", "logical_name": name, "canonical_sha256": "b" * 64}]}


class HistoricalSelectionTests(unittest.TestCase):
    def assert_code(self, code, operation):
        with self.assertRaises(WorkError) as caught:
            operation()
        self.assertEqual(caught.exception.code, code)

    def test_stored_metadata_does_not_require_unavailable_source_bytes(self):
        value = selection("task.unavailable")
        before = copy.deepcopy(value)
        self.assertEqual(stored_selection(value, selected_paths=[]), before)
        self.assertEqual(value, before)

    def test_duplicate_sources_and_selection_mismatch_are_rejected(self):
        value = selection()
        self.assert_code("work_instruction_selection_selected_paths_mismatch",
                         lambda: stored_selection(value, selected_paths=["web"]))
        value["sources"].append(copy.deepcopy(value["sources"][0]))
        self.assert_code("duplicate_instruction_source", lambda: stored_selection(value))

    def test_union_preserves_first_occurrence_order(self):
        first, second = selection(), selection("task.web", "c")
        first["references"], second["references"] = ["first"], ["second", "first"]
        document = {"sources": first["sources"] + second["sources"],
                    "references": ["first", "second"], "instructions_sha256": "d" * 64}
        self.assertEqual(stored_document_selection(document, [first, second, first]), document)
        document["sources"].reverse()
        self.assert_code("historical_instruction_union_mismatch",
                         lambda: stored_document_selection(document, [first, second]))

    def test_identical_sources_cannot_claim_different_fingerprints(self):
        first, second = selection(), selection(fingerprint="c")
        document = {key: first[key] for key in ("sources", "references", "instructions_sha256")}
        self.assert_code("historical_instruction_fingerprint_conflict",
                         lambda: stored_document_selection(document, [first, second]))

    def test_same_source_identity_cannot_have_conflicting_content(self):
        first, second = selection(), selection(fingerprint="c")
        second["sources"][0]["canonical_sha256"] = "d" * 64
        document = {key: first[key] for key in ("sources", "references", "instructions_sha256")}
        self.assert_code("instruction_source_identity_conflict",
                         lambda: stored_document_selection(document, [first, second]))
