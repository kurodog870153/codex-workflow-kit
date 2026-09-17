from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.contracts.progress import validate_progress_contract
from worklib.foundation.errors import WorkError


def discussion():
    return {
        "schema": "work-discussion-progress/v1", "requirement_id": "example",
        "mode": "task", "revision": 1, "status": "discussion_only",
        "title": "Discussion", "request": "Retain unfinished decisions",
        "current_task_id": "TASK-001", "context": {"missing_plan": True},
        "source_status": ["Revision pending"], "notes": ["Reviewed evidence"],
        "confirmed_decisions": [{"statement": "Keep scope", "rationale": "User decision"}],
        "tentative": ["Proposed approach"], "open_questions": ["Which validation?"],
        "next_discussion_point": "Confirm validation",
    }


class ProgressContractTests(unittest.TestCase):
    def test_historical_context_is_retained_without_aliasing_input(self):
        value = discussion()
        value["context"] = {"skill_selection": {"unavailable": True}, "command": "do not run"}
        before = copy.deepcopy(value)
        result = validate_progress_contract(value)
        self.assertEqual(result, before)
        result["context"]["skill_selection"]["unavailable"] = False
        self.assertEqual(value, before)

    def test_revision_and_role_boundaries(self):
        for changes, code in (
            ({"revision": True}, "invalid_progress_revision"),
            ({"revision": 0}, "invalid_progress_revision"),
            ({"revision": 1.5}, "invalid_progress_revision"),
            ({"mode": "execute"}, "invalid_progress_mode"),
            ({"mode": "plan"}, "invalid_progress_task"),
            ({"current_task_id": "TASK-1"}, "invalid_progress_task"),
            ({"status": "completed"}, "invalid_progress_schema"),
        ):
            with self.subTest(changes=changes):
                value = discussion()
                value.update(changes)
                with self.assertRaises(WorkError) as caught:
                    validate_progress_contract(value)
                self.assertEqual(caught.exception.code, code)

    def test_missing_unknown_and_empty_decision_fields_are_rejected(self):
        for decision in ({}, {"statement": ""}, {"statement": "Keep", "rationale": " "},
                         {"statement": "Keep", "approved": True}):
            with self.subTest(decision=decision):
                value = discussion()
                value["confirmed_decisions"] = [decision]
                with self.assertRaises(WorkError):
                    validate_progress_contract(value)

    def test_plan_without_task_and_optional_rationale_are_valid(self):
        value = discussion()
        value.update(mode="plan", current_task_id=None, confirmed_decisions=[{"statement": "Keep"}])
        self.assertEqual(validate_progress_contract(value), value)
