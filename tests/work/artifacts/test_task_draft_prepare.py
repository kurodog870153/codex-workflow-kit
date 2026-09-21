from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from artifacts import test_task_draft_sources as fixtures
from worklib.services.task.draft.storage import read_task_planning_index, save_task_planning, recover_task_planning
from worklib.business_services.task.draft_list import update_task_planning_list
from worklib.workflows.task import initialize_task_planning_request, prepare_task_planning_request
from worklib.business_services.plan import render_plan_contract
from worklib.models.common.errors import WorkError


class DraftPreparationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.TaskDraftSourceTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        entry = self.fixture.index["tasks"][0]
        self.boundary = {key: copy.deepcopy(entry[key]) for key in ("id", "title", "goal", "scope", "skill_id", "dependencies")}
        self.boundary["instruction_selection"] = {"selected_paths": [], "references": []}
        self.request = {"tasks": [self.boundary], "current_task_id": None}
        self.options = {"plan_path": self.fixture.plan["artifacts"]["plan"], "user_config_root": str(self.root)}

    def snapshot(self):
        return {p.relative_to(self.root).as_posix(): p.read_bytes() if p.is_file() else None for p in self.root.rglob("*")}

    def initialize(self, prepare_only=False):
        return initialize_task_planning_request(self.root, "example", self.request, prepare_only=prepare_only, **self.options)

    def edit(self, payload, revision=1):
        return prepare_task_planning_request(self.root, "example", payload, expected_revision=revision, **self.options)

    def test_initial_preview_and_save_derive_metadata_without_formal_artifacts(self):
        before = self.snapshot()
        preview = self.initialize(prepare_only=True)
        self.assertEqual(self.snapshot(), before)
        entry = preview["index"]["tasks"][0]
        self.assertEqual((entry["status"], entry["boundary_revision"]), ("planned", 1))
        self.assertEqual(entry["instructions_sha256"], self.fixture.index["tasks"][0]["instructions_sha256"])
        self.initialize()
        self.assertEqual(read_task_planning_index(self.root, "example"), preview["index"])
        self.assertFalse((self.root / self.fixture.plan["artifacts"]["task"]).exists())
        self.assertFalse((self.root / self.fixture.plan["artifacts"]["execution"]).exists())

    def test_initial_preview_accepts_collection_index_plan_artifact(self):
        self.fixture.plan["artifacts"]["task"] = "outputs/work/tasks/example/index.json"
        self.fixture.plan_path.write_bytes(render_plan_contract(self.fixture.plan))

        preview = self.initialize(prepare_only=True)

        self.assertEqual(preview["index"]["requirement_id"], "example")
        self.assertFalse((self.root / self.fixture.plan["artifacts"]["task"]).exists())

    def test_initial_invalid_inputs_leave_no_storage(self):
        original = copy.deepcopy(self.request)
        for defect in ("selection", "skill", "dependency", "metadata", "duplicate", "cycle"):
            with self.subTest(defect=defect):
                self.request = copy.deepcopy(original)
                entry = self.request["tasks"][0]
                if defect == "selection":
                    del entry["instruction_selection"]
                elif defect == "skill":
                    entry["skill_id"] = "unknown"
                elif defect == "dependency":
                    entry["dependencies"] = ["TASK-999"]
                elif defect == "metadata":
                    entry["status"] = "refined"
                elif defect == "duplicate":
                    self.request["tasks"].append(copy.deepcopy(entry))
                else:
                    entry["dependencies"] = ["TASK-002"]
                    other = copy.deepcopy(entry)
                    other.update(id="TASK-002", dependencies=["TASK-001"])
                    self.request["tasks"].append(other)
                before = self.snapshot()
                with self.assertRaises(WorkError):
                    self.initialize()
                self.assertEqual(self.snapshot(), before)

    def test_existing_and_reserved_storage_cannot_be_initialized(self):
        directory = self.root / "outputs/work/tasks/example/drafts/history/1"
        directory.mkdir(parents=True)
        before = self.snapshot()
        with self.assertRaises(WorkError) as caught:
            self.initialize()
        self.assertEqual(caught.exception.code, "draft_initial_storage_exists")
        self.assertEqual(self.snapshot(), before)

    def test_interrupted_init_exposes_exact_recovery_index(self):
        with patch("worklib.services.task.draft.storage.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError) as caught:
                self.initialize()
        index = caught.exception.details["prepared_index"]
        recover_task_planning(self.root, index, expected_revision=0)
        self.assertEqual(read_task_planning_index(self.root, "example"), index)

    def test_split_preview_retires_id_and_is_consumable_by_existing_update(self):
        self.initialize()
        first, second = copy.deepcopy(self.boundary), copy.deepcopy(self.boundary)
        first.update(id="TASK-002", title="First split")
        second.update(id="TASK-003", title="Second split", dependencies=["TASK-002"])
        payload = {"upsert": [first, second], "remove_task_ids": ["TASK-001"],
                   "current_task_id": "TASK-002", "reason": "Confirmed split"}
        before = self.snapshot()
        prepared = self.edit(payload)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(prepared["index"]["retired_task_ids"], ["TASK-001"])
        update_task_planning_list(self.root, prepared["request"]["index"], expected_revision=1, reason=payload["reason"])
        self.assertEqual(read_task_planning_index(self.root, "example"), prepared["index"])
        payload["upsert"] = [copy.deepcopy(self.boundary)]
        payload["remove_task_ids"] = []
        payload["current_task_id"] = "TASK-001"
        with self.assertRaises(WorkError):
            self.edit(payload, revision=2)

    def test_refined_history_and_unaffected_entries_are_preserved(self):
        for number in (2, 3):
            entry = copy.deepcopy(self.boundary)
            entry.update(id=f"TASK-{number:03d}", dependencies=["TASK-001"] if number == 2 else [])
            self.request["tasks"].append(entry)
        self.initialize()
        for number in (1, 2):
            index = read_task_planning_index(self.root, "example")
            expected = index["revision"]
            index["revision"] += 1
            entry = index["tasks"][number - 1]
            entry["status"] = "refined"
            draft = {"schema": "work-task-draft/v1", "requirement_id": "example", "task_id": entry["id"],
                     "revision": 1, "boundary_revision": 1, "source": index["source"],
                     "instructions_sha256": entry["instructions_sha256"], "status": "refined", "notes": ["Retain detail"],
                     "confirmed_decisions": [{"statement": "Keep decision", "rationale": "User confirmed"}],
                     "tentative": [], "open_questions": [], "next_discussion_point": None}
            save_task_planning(self.root, index, expected_revision=expected, draft=draft)
        before_index = read_task_planning_index(self.root, "example")
        changed = copy.deepcopy(self.boundary)
        changed["goal"] = "Confirmed revised goal"
        before = self.snapshot()
        prepared = self.edit({"upsert": [changed], "remove_task_ids": [], "current_task_id": None,
                              "reason": "Confirmed revision"}, revision=3)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(prepared["affected_task_ids"], ["TASK-001", "TASK-002"])
        self.assertEqual(prepared["index"]["tasks"][2], before_index["tasks"][2])
        self.assertEqual(prepared["drafts"]["TASK-002"]["confirmed_decisions"][0]["statement"], "Keep decision")
        self.assertEqual(prepared["index"]["tasks"][0]["boundary_revision"], 2)
        self.assertEqual(prepared["index"]["tasks"][1]["boundary_revision"], 1)

    def test_merge_preserves_unaffected_task_and_removes_old_dependencies(self):
        for number in (2, 3):
            entry = copy.deepcopy(self.boundary)
            entry.update(id=f"TASK-{number:03d}", dependencies=["TASK-001"] if number == 2 else [])
            self.request["tasks"].append(entry)
        self.initialize()
        previous = read_task_planning_index(self.root, "example")
        merged = copy.deepcopy(self.request["tasks"][1])
        merged.update(goal="Merged outcome", dependencies=[])
        result = self.edit({"upsert": [merged], "remove_task_ids": ["TASK-001"],
                            "current_task_id": "TASK-002", "reason": "Confirmed merge"})
        self.assertEqual(result["index"]["tasks"][1], previous["tasks"][2])
        self.assertEqual(result["index"]["retired_task_ids"], ["TASK-001"])

    def test_stale_revision_source_drift_and_selection_changes_are_rejected(self):
        self.initialize()
        changed = copy.deepcopy(self.boundary)
        changed["goal"] = "Changed"
        payload = {"upsert": [changed], "remove_task_ids": [], "current_task_id": None, "reason": "Review"}
        with self.assertRaises(WorkError):
            self.edit(payload, revision=2)
        changed["instruction_selection"]["references"] = ["task.general.task-records"]
        with self.assertRaises(WorkError) as caught:
            self.edit(payload)
        self.assertEqual(caught.exception.code, "draft_selection_mismatch")
        changed["instruction_selection"]["references"] = []
        self.fixture.plan["summary"] = "Changed Plan"
        self.fixture.plan_path.write_bytes(render_plan_contract(self.fixture.plan))
        with self.assertRaises(WorkError) as caught:
            self.edit(payload)
        self.assertEqual(caught.exception.code, "draft_source_drift")
