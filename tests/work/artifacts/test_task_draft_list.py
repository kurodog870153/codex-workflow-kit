from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.task_draft import read_task_draft, read_task_planning_index, save_task_planning
from worklib.artifacts.task_draft_list import update_task_planning_list
from worklib.foundation.errors import WorkError


class TaskDraftListTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        source = {"plan_sha256": "a" * 64, "hierarchy_selection_sha256": "b" * 64, "skill_selection_sha256": "c" * 64}
        index = {"schema": "work-task-planning-index/v1", "requirement_id": "example", "revision": 1, "current_task_id": "TASK-001", "source": source, "tasks": []}
        for number in range(1, 4):
            index["tasks"].append({"id": f"TASK-{number:03}", "title": "Task", "goal": "Result", "scope": ["Source"], "skill_id": None, "dependencies": ["TASK-001"] if number == 2 else [], "status": "planned", "boundary_revision": 1, "instructions_sha256": "d" * 64})
        save_task_planning(self.root, index, expected_revision=0)
        for position in range(3):
            index = read_task_planning_index(self.root, "example")
            expected = index["revision"]
            index["revision"] += 1
            index["tasks"][position]["status"] = "refined"
            draft = {"schema": "work-task-draft/v1", "requirement_id": "example", "task_id": f"TASK-{position + 1:03}", "revision": 1, "boundary_revision": 1, "source": source, "instructions_sha256": "d" * 64, "status": "refined", "notes": ["Detail"], "confirmed_decisions": [{"statement": "Decision", "rationale": "Reason"}], "tentative": [], "open_questions": [], "next_discussion_point": None}
            save_task_planning(self.root, index, expected_revision=expected, draft=draft)
        self.before = read_task_planning_index(self.root, "example")
        self.proposed = copy.deepcopy(self.before)
        self.proposed["revision"] += 1

    def update(self, recover=False):
        return update_task_planning_list(self.root, self.proposed, expected_revision=4, reason="Confirmed list change.", recover=recover)

    def test_boundary_change_marks_downstream_only_and_preserves_decisions(self):
        self.proposed["tasks"][0]["goal"] = "Changed result"
        result = self.update()
        self.assertEqual(result["affected_task_ids"], ["TASK-001", "TASK-002"])
        index = read_task_planning_index(self.root, "example")
        self.assertEqual(index["tasks"][2], self.before["tasks"][2])
        for task_id in ("TASK-001", "TASK-002"):
            discussion = read_task_draft(self.root, "example", task_id)
            self.assertEqual(discussion["status"], "needs_review")
            self.assertEqual(discussion["confirmed_decisions"][0]["statement"], "Decision")
        self.assertEqual(index["tasks"][0]["boundary_revision"], 2)
        self.assertEqual(index["tasks"][1]["boundary_revision"], 1)

    def test_split_retires_old_id_and_adds_new_boundaries(self):
        template = self.proposed["tasks"].pop(0)
        self.proposed["current_task_id"] = "TASK-004"
        self.proposed["tasks"][0]["dependencies"] = ["TASK-004", "TASK-005"]
        for task_id in ("TASK-004", "TASK-005"):
            entry = copy.deepcopy(template)
            entry.update(id=task_id, status="planned", boundary_revision=1)
            entry.pop("draft_ref")
            self.proposed["tasks"].append(entry)
        self.update()
        index = read_task_planning_index(self.root, "example")
        self.assertEqual(index["retired_task_ids"], ["TASK-001"])
        self.assertEqual(index["tasks"][0]["status"], "needs_review")
        reference = template["draft_ref"]
        self.assertTrue((self.root / f"outputs/work/tasks/example/drafts/history/{reference['save_revision']}/TASK-001.json").exists())

    def test_merge_preserves_unaffected_task(self):
        self.proposed["tasks"] = self.proposed["tasks"][1:]
        self.proposed["tasks"][0].update(goal="Combined result", dependencies=[])
        self.proposed["current_task_id"] = "TASK-002"
        self.update()
        index = read_task_planning_index(self.root, "example")
        self.assertEqual(index["tasks"][1], self.before["tasks"][2])
        self.assertEqual(index["retired_task_ids"], ["TASK-001"])

    def test_unchanged_or_forged_progress_is_rejected(self):
        with self.assertRaises(WorkError):
            self.update()
        self.proposed["tasks"][0].update(goal="Changed", status="needs_review")
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "draft_list_metadata_changed")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.before)

    def test_list_update_cannot_change_existing_selection(self):
        self.proposed["tasks"][0]["goal"] = "Changed goal"
        self.proposed["tasks"][0]["instruction_selection"] = {"selected_paths": [], "references": []}
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "draft_selection_mismatch")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.before)

    def test_interrupted_update_recovers_exact_content(self):
        self.proposed["tasks"][0]["goal"] = "Changed"
        with patch("worklib.artifacts.task_draft_list.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.update()
        self.assertEqual(read_task_planning_index(self.root, "example"), self.before)
        self.assertEqual(self.update(recover=True)["status"], "recovered")
        self.assertEqual(self.update(recover=True)["status"], "already_completed")

    def test_stale_update_is_rejected(self):
        self.proposed["tasks"][0]["goal"] = "Changed"
        self.update()
        with self.assertRaises(WorkError) as context:
            self.update()
        self.assertEqual(context.exception.code, "draft_revision_conflict")
