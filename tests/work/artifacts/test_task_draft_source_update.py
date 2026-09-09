from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.task_draft import save_task_planning, read_task_draft, read_task_planning_index
from worklib.artifacts.task_draft_source_update import update_task_draft_sources
from worklib.contracts.plan import render_plan_contract, validate_plan_file
from worklib.foundation.errors import ExitCode, WorkError
from worklib.hierarchy.selection import build_hierarchy_selection
from worklib.instructions.selection import build_instruction_selection
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.skills.selection import selection_sha256


class TaskDraftSourceUpdateTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        work_root = SCRIPT_ROOT.parent
        hierarchy = build_hierarchy_selection({"decision": "general_only", "selections": []}, skill_root=work_root)
        self.plan = {
            "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed",
            "title": "Plan", "summary": "Result",
            "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/task.json", "execution": "outputs/work/executions/example"},
            "hierarchy_selection": hierarchy,
            "work_instruction_selection": build_work_instruction_selection(skill_root=work_root, mode="plan", selected_paths=[]),
            "skill_selection": {"schema": "work-skill-selection/v1", "decision": "base_only", "skills": [], "selection_sha256": selection_sha256("base_only", [])},
            "goals": [{"id": "GOAL-001", "statement": "Result"}],
            "scope": [{"id": "SCOPE-001", "kind": "in_scope", "statement": "Source", "goal_ids": ["GOAL-001"]}],
            "deliverables": [{"id": "DELIVERABLE-001", "statement": "Result", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}],
            "acceptance_criteria": [{"id": "ACCEPTANCE-001", "statement": "Observable", "deliverable_ids": ["DELIVERABLE-001"]}],
        }
        self.plan_path = self.root / self.plan["artifacts"]["plan"]
        self.plan_path.parent.mkdir(parents=True)
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        checked = validate_plan_file(self.root, str(self.root), self.plan["artifacts"]["plan"])
        selection = build_instruction_selection(skill_root=work_root, mode="task", selected_paths=[], reference_names=[])
        self.index = {
            "schema": "work-task-planning-index/v1", "requirement_id": "example", "revision": 1,
            "current_task_id": "TASK-001",
            "source": {key: checked[key] for key in ("plan_sha256", "hierarchy_selection_sha256", "skill_selection_sha256")},
            "tasks": [{"id": "TASK-001", "title": "Task", "goal": "Result", "scope": ["Source"], "skill_id": None,
                       "dependencies": [], "status": "planned", "boundary_revision": 1, "instructions_sha256": selection["instructions_sha256"]}],
        }

        self.request = {"reason": "Confirmed source change", "selections": {"TASK-001": {"selected_paths": [], "references": []}}}
        self.index["tasks"][0]["instruction_selection"] = {"selected_paths": [], "references": []}
        self.options = {"expected_revision": 2, "plan_path": self.plan["artifacts"]["plan"], "user_config_root": str(self.root)}

    def initialize(self):
        save_task_planning(self.root, self.index, expected_revision=0)
        index = copy.deepcopy(self.index)
        index["revision"] = 2
        index["tasks"][0]["status"] = "refined"
        draft = {
            "schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001",
            "revision": 1, "boundary_revision": 1, "source": index["source"],
            "instructions_sha256": index["tasks"][0]["instructions_sha256"], "status": "refined",
            "notes": ["Original evidence"], "confirmed_decisions": [{"statement": "Decision", "rationale": "Reason"}],
            "tentative": [], "open_questions": [], "next_discussion_point": None,
        }
        save_task_planning(self.root, index, expected_revision=1, draft=draft)
        self.before = read_task_planning_index(self.root, "example")

    def update(self, recover=False):
        return update_task_draft_sources(self.root, "example", self.request, recover=recover, **self.options)

    def change_plan(self):
        self.plan["summary"] = "Changed result"
        self.plan_path.write_bytes(render_plan_contract(self.plan))

    def test_plan_refresh_preserves_evidence_and_marks_review(self):
        self.initialize()
        self.change_plan()
        result = self.update()
        self.assertEqual(result["affected_task_ids"], ["TASK-001"])
        index = read_task_planning_index(self.root, "example")
        self.assertNotEqual(index["source"], self.before["source"])
        draft = read_task_draft(self.root, "example", "TASK-001")
        self.assertEqual(draft["status"], "needs_review")
        self.assertEqual(draft["notes"][0], "Original evidence")
        self.assertEqual(draft["confirmed_decisions"][0]["rationale"], "Reason")
        self.assertEqual(draft["source"], index["source"])

    def test_source_update_records_selection_for_legacy_entry(self):
        self.index["tasks"][0].pop("instruction_selection")
        self.initialize()
        self.change_plan()
        self.update()
        entry = read_task_planning_index(self.root, "example")["tasks"][0]
        self.assertEqual(entry["instruction_selection"], self.request["selections"]["TASK-001"])
        self.assertNotIn("instruction_selection", self.before["tasks"][0])

    def test_local_instruction_update_preserves_unaffected_reference(self):
        second = copy.deepcopy(self.index["tasks"][0])
        second["id"] = "TASK-002"
        self.index["tasks"].append(second)
        self.request["selections"]["TASK-002"] = {"selected_paths": [], "references": []}
        self.initialize()
        self.request["selections"]["TASK-001"]["references"] = ["task.general.task-records"]
        self.assertEqual(self.update()["affected_task_ids"], ["TASK-001"])
        index = read_task_planning_index(self.root, "example")
        self.assertEqual(index["tasks"][1], self.before["tasks"][1])
        self.assertEqual(index["tasks"][0]["boundary_revision"], 2)
        self.assertEqual(index["tasks"][0]["instruction_selection"]["references"], ["task.general.task-records"])

    def test_update_requires_current_revision(self):
        self.initialize()
        self.change_plan()
        self.options["expected_revision"] = 1
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "draft_revision_conflict")

    def test_no_change_does_not_write(self):
        self.initialize()
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "draft_sources_unchanged")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.before)

    def test_rejects_missing_selection_and_unknown_skill(self):
        self.index["tasks"][0]["skill_id"] = "missing"
        self.initialize()
        self.change_plan()
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "draft_skill_not_available")
        self.request["selections"] = {}
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "invalid_object_fields")

    def test_interrupted_refresh_can_recover_identical_sources(self):
        self.initialize()
        self.change_plan()
        with patch("worklib.artifacts.task_draft_source_update.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.update()
        self.assertEqual(read_task_planning_index(self.root, "example"), self.before)
        self.assertEqual(self.update(recover=True)["status"], "recovered")
        self.assertEqual(self.update(recover=True)["status"], "already_completed")
        self.assertEqual(read_task_planning_index(self.root, "example")["tasks"][0]["instruction_selection"], self.request["selections"]["TASK-001"])

    def test_recovery_rejects_sources_changed_again(self):
        self.initialize()
        self.change_plan()
        with patch("worklib.artifacts.task_draft_source_update.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.update()
        self.plan["summary"] = "Another change"
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        with self.assertRaises(WorkError) as context:
            self.update(recover=True)
        self.assertEqual(context.exception.code, "draft_recovery_conflict")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.before)
