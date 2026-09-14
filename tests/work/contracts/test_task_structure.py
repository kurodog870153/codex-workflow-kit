from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.contracts.task_structure import inspect_task_structure, pointer


class TaskStructureTests(unittest.TestCase):
    def test_pointer_escapes_slashes_and_tildes(self):
        self.assertEqual(pointer("/tasks/0", "a~/b"), "/tasks/0/a~0~1b")

    def test_independent_issues_preserve_input_and_decision_categories(self):
        value = {"title": " ", "tasks": [{"goal": "", "rules_sha256": "old"}], "a~/b": 1}
        before = copy.deepcopy(value)
        issues = inspect_task_structure(value)
        by_location = {item["location"]: item for item in issues}
        self.assertEqual(by_location["/title"]["category"], "decision_required")
        self.assertEqual(by_location["/tasks/0/goal"]["code"], "empty_text")
        self.assertEqual(by_location["/tasks/0/rules_sha256"]["category"], "migration_review")
        self.assertEqual(by_location["/a~0~1b"]["code"], "unknown_field")
        self.assertEqual(value, before)

    def test_conditional_fields_are_reported_at_exact_locations(self):
        value = {"tasks": [{"commands": [{"id": "CMD-001", "mode": "argv"}],
                            "files": [{"id": "FILE-001", "action": "move"}],
                            "validations": [{"id": "VAL-001", "kind": "manual"}]}]}
        missing = {item["location"] for item in inspect_task_structure(value)
                   if item["code"] == "missing_field"}
        for path in ("commands/0/argv", "files/0/source", "files/0/destination",
                     "validations/0/confirmer", "validations/0/criteria"):
            self.assertIn("/tasks/0/" + path, missing)

    def test_wrong_nested_types_do_not_hide_other_issues(self):
        issues = inspect_task_structure({"tasks": [None, {"steps": "not an array", "validations": []}]})
        observed = {(item["location"], item["code"]) for item in issues}
        self.assertIn(("/tasks/0", "invalid_type"), observed)
        self.assertIn(("/tasks/1/steps", "invalid_type"), observed)
        self.assertIn(("/tasks/1/validations", "empty_array"), observed)
