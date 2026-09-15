from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from artifacts import test_specification as fixtures
from worklib.artifacts.migration_prepare import prepare_migration
from worklib.foundation.errors import WorkError
from worklib.cli import main
from io import StringIO


class MigrationPreparationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)

    def request(self, repair=False):
        f = self.fixture
        full = f.migration_repair_request() if repair else f.migration_request()
        def choice(selection):
            return {key: selection[key] for key in ("selected_paths", "references")}
        request = {"schema": "work-migration-prepare-request/v1", "plan_path": f.artifacts["plan"],
            "reason": full["reason"], "edits": [], "instruction_review": full["instruction_review"],
            "instruction_choices": {"plan": choice(full["plan"]["work_instruction_selection"]),
                "tasks": {row["id"]: choice(row["instruction_selection"]) for row in full["task"]["tasks"]}}}
        original = json.loads(f.task_path.read_bytes())
        for row in original["tasks"]:
            after = copy.deepcopy(row["validations"])
            after[0]["criteria"] += "; reviewed against current guidance"
            request["edits"].append({"artifact": "task", "task_id": row["id"], "field": "validations",
                                     "before": row["validations"], "after": after})
        if repair:
            request["source_plan_repair"] = full["source_plan_repair"]
        return request

    def prepare(self, request):
        return prepare_migration(json.dumps(request).encode(), project_root=self.fixture.root,
                                 user_config_root=str(self.fixture.root))

    def test_candidates_revalidate_with_identical_approval_without_writes(self):
        request = self.request()
        before = self.fixture.snapshot()
        result = self.prepare(request)
        self.assertEqual(result["schema"], "work-migration-prepare/v1")
        self.assertEqual(self.fixture.run_migration(result["request"]), result["preview"])
        self.assertEqual(result["request"]["task"]["spec_id"], "TASK-SPEC-002")
        self.assertTrue(result["preview"]["migration"]["plan_edits"])
        plan = result["request"]["plan"]
        self.assertEqual(plan["changes"][-1]["affected_ids"], [
            item["id"] for group in ("goals", "scope", "deliverables", "acceptance_criteria")
            for item in plan[group]
        ])
        self.assertEqual(before, self.fixture.snapshot())
        self.assertEqual(result["transport"]["request_schema"], "work-spec-migration-request/v1")
        self.assertEqual(result["next_step"], {"command": "task migrate-validate", "input": "request"})

    def test_explicit_binding_repair_is_preserved(self):
        request = self.request(repair=True)
        before = self.fixture.snapshot()
        result = self.prepare(request)
        self.assertEqual(result["request"]["source_plan_repair"], request["source_plan_repair"])
        self.assertEqual(self.fixture.run_migration(result["request"]), result["preview"])
        self.assertEqual(before, self.fixture.snapshot())

    def test_invalid_review_choices_and_stale_edits_are_rejected(self):
        request = self.request()
        candidates = []
        for field in ("plan", "task", "execute"):
            bad = copy.deepcopy(request)
            bad["instruction_review"][field] = ""
            candidates.append(bad)
        bad = copy.deepcopy(request)
        bad["instruction_choices"]["tasks"].pop("TASK-001")
        candidates.append(bad)
        bad = copy.deepcopy(request)
        bad["edits"][0]["before"] = []
        candidates.append(bad)
        before = self.fixture.snapshot()
        for bad in candidates:
            with self.subTest(request=bad), self.assertRaises(WorkError):
                self.prepare(bad)
            self.assertEqual(before, self.fixture.snapshot())

    def test_task_affected_ids_reports_exact_migration_edit(self):
        request = self.request()
        request["edits"][1]["affected_ids"] = ["TASK-002"]
        with self.assertRaises(WorkError) as caught:
            self.prepare(request)
        self.assertEqual(caught.exception.code, "spec_prepare_task_affected_ids")
        self.assertEqual(caught.exception.details["edit_index"], 1)
        self.assertEqual(caught.exception.details["task_id"], "TASK-002")
        self.assertEqual(caught.exception.details["field"], "validations")

    def test_missing_and_stale_repair_evidence_are_rejected(self):
        request = self.request(repair=True)
        for key in (None, "actual_sha256", "recorded_sha256", "review"):
            bad = copy.deepcopy(request)
            if key is None:
                del bad["source_plan_repair"]
            else:
                bad["source_plan_repair"][key] = "" if key == "review" else "0" * 64
            with self.subTest(key=key), self.assertRaises(WorkError):
                self.prepare(bad)

    def test_source_drift_during_validation_is_rejected(self):
        request = self.request()
        from worklib.artifacts.specification import update_specification
        def changed(*args, **kwargs):
            self.fixture.plan_path.write_bytes(self.fixture.plan_path.read_bytes() + b"\n")
            return update_specification(*args, **kwargs)
        with patch("worklib.artifacts.spec_prepare.update_specification", side_effect=changed):
            with self.assertRaises(WorkError) as caught:
                self.prepare(request)
        self.assertEqual(caught.exception.code, "spec_update_source_changed")

    def test_cli_dispatch(self):
        request = self.request()
        out, err = StringIO(), StringIO()
        code = main(self.fixture.input_arguments(["--project-root", str(self.fixture.root),
            "task", "migrate-prepare", "--input-file", "request.json", "--user-config-root",
            str(self.fixture.root)], json.dumps(request)), stdout=out, stderr=err)
        self.assertEqual(code, 0, err.getvalue())
        self.assertEqual(json.loads(out.getvalue())["data"]["schema"], "work-migration-prepare/v1")

    def test_active_writer_and_incomplete_transaction_block_preparation(self):
        request = self.request()
        from worklib.foundation.spec_update import state_writer
        with state_writer(self.fixture.root, self.fixture.artifacts["execution"]):
            with self.assertRaises(WorkError):
                self.prepare(request)
        record = self.fixture.index_path.parent / ".work-spec-update-001.json"
        record.write_text("{}")
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError):
            self.prepare(request)
        self.assertEqual(before, self.fixture.snapshot())
