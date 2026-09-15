from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from artifacts import test_specification as fixtures
from worklib.artifacts.spec_prepare import prepare_specification
from worklib.contracts.execution_index import build_initial_execution_index, render_execution_index
from worklib.contracts.plan import render_plan_contract
from worklib.contracts.task import render_task_contract, validate_task_contract
from worklib.foundation.fingerprint import raw_sha256
from worklib.foundation.errors import WorkError


class SpecificationPreparationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)

    def prepare(self, request):
        return prepare_specification(json.dumps(request).encode("utf-8"),
            project_root=self.fixture.root, user_config_root=str(self.fixture.root))

    def test_duplicate_and_unchanged_edits_are_rejected_without_writes(self):
        for duplicate, code in ((True, "spec_prepare_duplicate"), (False, "spec_prepare_unchanged")):
            with self.subTest(duplicate=duplicate):
                request = self.fixture.prepare_request()
                if duplicate:
                    request["edits"].append(copy.deepcopy(request["edits"][0]))
                else:
                    request["edits"][0]["after"] = request["edits"][0]["before"]
                before = self.fixture.snapshot()
                with self.assertRaises(WorkError) as caught:
                    self.prepare(request)
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(self.fixture.snapshot(), before)

    def test_missing_plan_change_evidence_is_rejected(self):
        request = self.fixture.prepare_request()
        request["edits"] = [{"artifact": "plan", "field": "summary",
                             "before": "Original result", "after": "Confirmed result"}]
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError) as caught:
            self.prepare(request)
        self.assertEqual(caught.exception.code, "spec_prepare_plan_evidence")
        self.assertEqual(self.fixture.snapshot(), before)

    def test_prepared_transport_revalidates_to_identical_approval(self):
        before = self.fixture.snapshot()
        result = self.prepare(self.fixture.prepare_request())
        transported = json.loads(json.dumps(result["request"], ensure_ascii=False))
        self.assertEqual(self.fixture.run_update(transported), result["preview"])
        self.assertEqual(self.fixture.snapshot(), before)
        self.assertEqual(result["request"]["task"]["tasks"][2], self.fixture.task["tasks"][2])
        self.assertEqual(result["transport"], {
            "request_field": "request", "request_schema": "work-spec-update-request/v1",
            "output_file": None,
        })
        self.assertEqual(result["next_step"], {"command": "task spec-validate", "input": "request"})

    def test_optional_plan_groups_are_editable(self):
        expected = {"constraints", "dependencies", "risks", "milestones", "decisions"}
        from worklib.artifacts.spec_prepare import PLAN_FIELDS
        self.assertTrue(expected <= PLAN_FIELDS)
        constraint = {"id": "CONSTRAINT-001", "statement": "Original constraint",
                      "applies_to": ["GOAL-001"]}
        self.fixture.plan["constraints"] = [constraint]
        self.fixture.plan_path.write_bytes(render_plan_contract(self.fixture.plan))
        self.fixture.task["source_plan"]["canonical_sha256"] = raw_sha256(self.fixture.plan_path.read_bytes())
        self.fixture.task_path.write_bytes(render_task_contract(self.fixture.task))
        validation = validate_task_contract(
            self.fixture.task_path.read_bytes(), source="fixture",
            actual_task_path=self.fixture.artifacts["task"], project_root=self.fixture.root,
            user_config_root=str(self.fixture.root),
        )
        self.fixture.index = build_initial_execution_index(self.fixture.task, validation)
        self.fixture.index_path.write_bytes(render_execution_index(self.fixture.index))
        request = self.fixture.prepare_request()
        changed = copy.deepcopy(constraint)
        changed["statement"] = "Confirmed constraint"
        request["edits"] = [{"artifact": "plan", "field": "constraints",
                             "before": [constraint], "after": [changed],
                             "affected_ids": ["CONSTRAINT-001"]}]
        result = self.prepare(request)
        self.assertEqual(result["request"]["plan"]["constraints"], [changed])
        self.assertEqual(result["preview"]["affected_task_ids"], ["TASK-001", "TASK-002", "TASK-003"])

    def test_invalid_edit_reports_exact_location_and_allowed_fields(self):
        cases = (
            ({"artifact": "index"}, "spec_prepare_artifact"),
            ({"field": "id"}, "spec_prepare_field"),
            ({"affected_ids": ["TASK-001"]}, "spec_prepare_task_affected_ids"),
        )
        for update, code in cases:
            with self.subTest(update=update):
                request = self.fixture.prepare_request()
                request["edits"][0].update(update)
                with self.assertRaises(WorkError) as caught:
                    self.prepare(request)
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(caught.exception.details["edit_index"], 0)
                self.assertEqual(caught.exception.details["location"], "edits[0]")
                if code == "spec_prepare_field":
                    self.assertIn("goal", caught.exception.details["allowed_fields"])

    def test_stale_before_reports_field_fingerprint_without_value(self):
        request = self.fixture.prepare_request()
        request["edits"][0]["before"] = "Stale goal"
        with self.assertRaises(WorkError) as caught:
            self.prepare(request)
        self.assertEqual(caught.exception.code, "spec_prepare_old_value")
        self.assertEqual(caught.exception.details["location"], "edits[0]")
        self.assertEqual(caught.exception.details["field"], "goal")
        self.assertEqual(len(caught.exception.details["actual_sha256"]), 64)
        self.assertNotIn("actual", caught.exception.details)
