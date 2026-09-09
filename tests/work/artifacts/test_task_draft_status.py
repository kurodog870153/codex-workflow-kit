from __future__ import annotations

import copy
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.task_draft import read_task_planning_index, save_task_planning
from worklib.artifacts.task_draft_status import task_draft_status
from worklib.cli import main
from worklib.foundation.errors import ExitCode, WorkError


class TaskDraftStatusTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.storage = self.root / "outputs/work/tasks/example/drafts"
        self.index = {
            "schema": "work-task-planning-index/v1", "requirement_id": "example", "revision": 1,
            "current_task_id": "TASK-001",
            "source": {"plan_sha256": "a" * 64, "hierarchy_selection_sha256": "b" * 64, "skill_selection_sha256": "c" * 64},
            "tasks": [{"id": task_id, "title": "Task", "goal": "Result", "scope": ["Source"], "skill_id": None,
                       "dependencies": [], "status": "planned", "boundary_revision": 1, "instructions_sha256": "d" * 64}
                      for task_id in ("TASK-001", "TASK-002")],
        }

    def initialize(self):
        save_task_planning(self.root, self.index, expected_revision=0)

    def save_discussion(self, task_id, status):
        index = read_task_planning_index(self.root, "example")
        expected = index["revision"]
        entry = next(task for task in index["tasks"] if task["id"] == task_id)
        draft = {
            "schema": "work-task-draft/v1", "requirement_id": "example", "task_id": task_id,
            "revision": entry.get("draft_ref", {}).get("revision", 0) + 1,
            "boundary_revision": 1, "source": copy.deepcopy(index["source"]),
            "instructions_sha256": "d" * 64, "status": status, "notes": ["具體討論"],
            "confirmed_decisions": [], "tentative": [],
            "open_questions": [] if status == "refined" else ["Which test?"],
            "next_discussion_point": None if status == "refined" else "Confirm test.",
        }
        entry["status"] = status
        index["revision"] += 1
        return save_task_planning(self.root, index, expected_revision=expected, draft=draft)

    def snapshot(self):
        return {path.relative_to(self.root): path.read_bytes() if path.is_file() else None for path in self.root.rglob("*")}

    def status(self, task_id=None):
        before = self.snapshot()
        try:
            return task_draft_status(self.root, "example", task_id=task_id)
        finally:
            self.assertEqual(self.snapshot(), before)

    def test_missing_index_proposes_confirmation_without_creating_storage(self):
        result = self.status()
        self.assertEqual((result["status"], result["next_action"]), ("not_initialized", "confirm_task_list"))
        self.assertTrue(result["requires_user_confirmation"])
        self.assertFalse(self.storage.exists())

    def test_missing_index_with_residue_requires_inspection(self):
        (self.storage / "history/1").mkdir(parents=True)
        result = self.status()
        self.assertEqual((result["status"], result["next_action"]), ("recovery_required", "inspect_recovery"))

    def test_invalid_index_is_not_treated_as_uninitialized(self):
        self.storage.mkdir(parents=True)
        (self.storage / "index.json").write_bytes(b"broken")
        with self.assertRaises(WorkError):
            self.status()

    def test_planned_current_task_is_proposed_without_live_source_claims(self):
        self.initialize()
        result = self.status()
        self.assertEqual(result["next_action"], "confirm_start")
        self.assertEqual(result["selected_task_id"], "TASK-001")
        self.assertEqual(result["counts"]["planned"], 2)
        self.assertEqual(result["source_validation"], "not_checked")
        self.assertIn("task draft-check", result["required_checks"])
        self.assertIsNone(result["instruction_selection"])
        self.assertTrue(result["selection_confirmation_required"])

    def test_status_returns_stored_selection_without_live_validation(self):
        selected = {"selected_paths": ["web/backend"], "references": ["task.general.task-records"]}
        self.index["tasks"][0]["instruction_selection"] = selected
        self.initialize()
        result = self.status()
        self.assertEqual(result["instruction_selection"], selected)
        self.assertFalse(result["selection_confirmation_required"])
        self.assertEqual(result["source_validation"], "not_checked")

    def test_saved_states_route_to_resume_or_review(self):
        self.initialize()
        for state, action in (("in_progress", "confirm_resume"), ("needs_review", "confirm_review")):
            self.save_discussion("TASK-001", state)
            result = self.status()
            self.assertEqual(result["next_action"], action)
            self.assertEqual(result["discussion"]["open_questions"], ["Which test?"])
            self.assertEqual(result["discussion"]["next_discussion_point"], "Confirm test.")

    def test_refined_current_task_does_not_automatically_advance(self):
        self.initialize()
        self.save_discussion("TASK-001", "refined")
        result = self.status()
        self.assertEqual(result["next_action"], "choose_task")
        self.assertEqual(result["selected_task_id"], "TASK-001")
        selected = self.status("TASK-002")
        self.assertEqual(selected["next_action"], "confirm_start")
        self.assertEqual(selected["current_task_id"], "TASK-001")

    def test_no_current_task_requires_choice(self):
        self.index["current_task_id"] = None
        self.initialize()
        result = self.status()
        self.assertEqual(result["next_action"], "choose_task")
        self.assertIsNone(result["selected_task_id"])

    def test_all_refined_only_proposes_assembly_even_without_candidates(self):
        self.initialize()
        for task_id in ("TASK-001", "TASK-002"):
            self.save_discussion(task_id, "refined")
        result = self.status()
        self.assertEqual(result["next_action"], "assemble_for_review")
        self.assertEqual(result["required_checks"], ["task draft-assemble"])
        self.assertFalse(result["discussion"]["has_task_candidate"])
        self.assertEqual(result["assembly_validation"], "not_performed")

    def test_reads_only_selected_history_and_ignores_display_copy(self):
        self.initialize()
        self.save_discussion("TASK-001", "in_progress")
        self.save_discussion("TASK-002", "in_progress")
        (self.storage / "history/3/TASK-002.json").write_bytes(b"unrelated corrupt discussion")
        (self.storage / "TASK-001.json").write_bytes(b"stale display copy")
        self.assertEqual(self.status()["discussion"]["notes"], ["具體討論"])
        with self.assertRaises(WorkError) as context:
            self.status("TASK-002")
        self.assertEqual(context.exception.code, "draft_content_integrity")

    def test_unknown_task_is_rejected(self):
        self.initialize()
        with self.assertRaises(WorkError) as context:
            self.status("TASK-999")
        self.assertEqual(context.exception.code, "draft_task_not_in_index")

    def test_reserved_next_revision_requires_inspection(self):
        self.initialize()
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=OSError("busy")):
            with self.assertRaises(WorkError):
                self.save_discussion("TASK-001", "in_progress")
        result = self.status()
        self.assertEqual(result["revision"], 1)
        self.assertEqual((result["status"], result["next_action"]), ("recovery_required", "inspect_recovery"))

    def test_concurrent_index_change_is_rejected(self):
        self.initialize()
        changed = {**self.index, "revision": 2}
        with patch("worklib.artifacts.task_draft_status.read_task_planning_index", side_effect=[self.index, changed]):
            with self.assertRaises(WorkError) as context:
                self.status()
        self.assertEqual(context.exception.code, "draft_revision_conflict")

    def test_unreadable_storage_is_not_reported_as_missing(self):
        with patch("pathlib.Path.stat", side_effect=PermissionError("denied")):
            with self.assertRaises((WorkError, PermissionError)):
                task_draft_status(self.root, "example")
        self.assertFalse(self.storage.exists())

    def test_cli_reports_progress_without_writing(self):
        self.initialize()
        before = self.snapshot()
        output, error = io.StringIO(), io.StringIO()
        code = main(["--project-root", str(self.root), "task", "draft-status", "--requirement-id", "example"], stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output.getvalue())["next_action"], "confirm_start")
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
