from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.task_draft import (
    read_task_draft, read_task_planning_index, recover_task_planning, save_task_planning,
)
from worklib.foundation.errors import WorkError


class TaskDraftArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.storage = self.root / "outputs/work/tasks/example/drafts"
        self.index = {
            "schema": "work-task-planning-index/v1", "requirement_id": "example",
            "revision": 1, "current_task_id": "TASK-001",
            "source": {"plan_sha256": "a" * 64, "hierarchy_selection_sha256": "b" * 64, "skill_selection_sha256": "c" * 64},
            "tasks": [{
                "id": "TASK-001", "title": "Source", "goal": "Update source.",
                "scope": ["Source only."], "skill_id": None, "dependencies": [],
                "status": "planned", "boundary_revision": 1, "instructions_sha256": "d" * 64,
            }],
        }
        self.draft = {
            "schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001",
            "revision": 1, "boundary_revision": 1, "source": copy.deepcopy(self.index["source"]),
            "instructions_sha256": "d" * 64, "status": "in_progress", "notes": ["保留討論"],
            "confirmed_decisions": [], "tentative": [], "open_questions": ["Which test?"],
            "next_discussion_point": "Confirm test.",
        }

    def initialize(self) -> None:
        save_task_planning(self.root, self.index, expected_revision=0)

    def save(self):
        proposed = read_task_planning_index(self.root, "example")
        expected = proposed["revision"]
        proposed["revision"] += 1
        proposed["tasks"][0]["status"] = self.draft["status"]
        return save_task_planning(self.root, proposed, expected_revision=expected, draft=self.draft)

    def test_initial_index_and_discussion_round_trip(self) -> None:
        self.initialize()
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)
        self.assertEqual(self.save()["mirror_status"], "updated")
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001"), self.draft)
        self.assertEqual((self.storage / "TASK-001.json").read_bytes(), (self.storage / "history/2/TASK-001.json").read_bytes())
        self.assertFalse((self.storage.parent / "task.md").exists())
        self.assertFalse((self.root / "outputs/work/executions").exists())

    def test_updates_preserve_immutable_history(self) -> None:
        self.initialize()
        self.save()
        original = (self.storage / "history/2/TASK-001.json").read_bytes()
        self.draft["revision"] = 2
        self.draft["notes"].append("More detail.")
        self.save()
        self.assertEqual((self.storage / "history/2/TASK-001.json").read_bytes(), original)
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001"), self.draft)

    def test_read_uses_history_and_never_display_copy(self) -> None:
        self.initialize()
        self.save()
        (self.storage / "TASK-001.json").write_bytes(b"broken display copy")
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001"), self.draft)

    def test_only_selected_draft_is_read_and_old_reference_is_reused(self) -> None:
        second = copy.deepcopy(self.index["tasks"][0])
        second["id"] = "TASK-002"
        self.index["tasks"].append(second)
        self.initialize()
        self.save()
        proposed = read_task_planning_index(self.root, "example")
        reference = copy.deepcopy(proposed["tasks"][0]["draft_ref"])
        proposed["revision"] = 3
        proposed["tasks"][1]["status"] = "in_progress"
        other = {**self.draft, "task_id": "TASK-002"}
        save_task_planning(self.root, proposed, expected_revision=2, draft=other)
        self.assertEqual(read_task_planning_index(self.root, "example")["tasks"][0]["draft_ref"], reference)
        self.assertFalse((self.storage / "history/3/TASK-001.json").exists())
        original_read = Path.read_bytes
        touched = []
        def recording_read(path):
            touched.append(path)
            return original_read(path)
        with patch.object(Path, "read_bytes", recording_read):
            self.assertEqual(read_task_draft(self.root, "example", "TASK-001"), self.draft)
        self.assertFalse(any(path.name == "TASK-002.json" for path in touched))

    def test_stale_writer_cannot_overwrite_newer_index(self) -> None:
        self.initialize()
        stale = copy.deepcopy(self.index)
        stale["revision"] = 2
        stale["tasks"][0]["status"] = "in_progress"
        self.save()
        before = (self.storage / "index.json").read_bytes()
        with self.assertRaises(WorkError) as context:
            save_task_planning(self.root, stale, expected_revision=1, draft=self.draft)
        self.assertEqual(context.exception.code, "draft_revision_conflict")
        self.assertEqual((self.storage / "index.json").read_bytes(), before)

    def test_interruption_before_commit_preserves_previous_index(self) -> None:
        self.initialize()
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError) as context:
                self.save()
        self.assertTrue(context.exception.details["recovery_required"])
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)
        with self.assertRaises(WorkError):
            self.save()
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)

    def test_partial_history_write_does_not_publish_index(self) -> None:
        self.initialize()
        with patch("worklib.artifacts.task_draft._write", side_effect=OSError("disk full")):
            with self.assertRaises(WorkError):
                self.save()
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)

    def test_display_failure_reports_committed_save(self) -> None:
        import os
        self.initialize()
        replace = os.replace
        def fail_display(source, destination):
            if destination.name == "TASK-001.json":
                raise OSError("display unavailable")
            return replace(source, destination)
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=fail_display):
            self.assertEqual(self.save()["mirror_status"], "stale")
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001"), self.draft)

    def test_corrupt_historical_draft_is_rejected(self) -> None:
        self.initialize()
        self.save()
        (self.storage / "history/2/TASK-001.json").write_bytes(b"{}\n")
        with self.assertRaises(WorkError) as context:
            read_task_draft(self.root, "example", "TASK-001")
        self.assertEqual(context.exception.code, "draft_content_integrity")

    def test_corrupt_index_history_is_rejected(self) -> None:
        self.initialize()
        (self.storage / "history/1/index.json").write_bytes(b"{}\n")
        with self.assertRaises(WorkError) as context:
            read_task_planning_index(self.root, "example")
        self.assertEqual(context.exception.code, "draft_index_integrity")

    def test_paths_and_links_are_rejected_before_writing(self) -> None:
        for requirement in ("../escape", "C:/escape", "con"):
            with self.subTest(requirement=requirement), self.assertRaises(WorkError):
                read_task_planning_index(self.root, requirement)
        with patch.object(Path, "is_symlink", return_value=True):
            with self.assertRaises(WorkError) as context:
                self.initialize()
        self.assertEqual(context.exception.code, "draft_path_link")
        self.assertFalse(self.storage.exists())

    def test_unversioned_boundary_change_is_rejected(self) -> None:
        self.initialize()
        proposed = copy.deepcopy(self.index)
        proposed["revision"] = 2
        proposed["tasks"][0].update(status="in_progress", goal="Changed goal.")
        with self.assertRaises(WorkError) as context:
            save_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        self.assertEqual(context.exception.code, "draft_boundary_conflict")

    def interrupted_proposal(self):
        self.initialize()
        proposed = copy.deepcopy(self.index)
        proposed["revision"] = 2
        proposed["tasks"][0]["status"] = "in_progress"
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                save_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        return proposed

    def test_recovery_publishes_identical_prepared_save_and_is_idempotent(self) -> None:
        proposed = self.interrupted_proposal()
        history = self.storage / "history/2/TASK-001.json"
        before = history.read_bytes()
        result = recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        self.assertEqual(result["status"], "recovered")
        self.assertEqual(history.read_bytes(), before)
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001"), self.draft)
        self.assertEqual(recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)["status"], "already_completed")

    def test_recovery_of_initial_index(self) -> None:
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.initialize()
        self.assertEqual(recover_task_planning(self.root, self.index, expected_revision=0)["status"], "recovered")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)

    def test_recovery_rejects_changed_request_without_writing(self) -> None:
        proposed = self.interrupted_proposal()
        changed = copy.deepcopy(self.draft)
        changed["notes"].append("Not the approved content.")
        before = {p: p.read_bytes() for p in self.storage.rglob("*") if p.is_file()}
        with self.assertRaises(WorkError) as context:
            recover_task_planning(self.root, proposed, expected_revision=1, draft=changed)
        self.assertEqual(context.exception.code, "draft_recovery_conflict")
        self.assertEqual(before, {p: p.read_bytes() for p in self.storage.rglob("*") if p.is_file()})

    def test_recovery_rejects_unknown_or_partial_files(self) -> None:
        proposed = self.interrupted_proposal()
        prepared = self.storage / "history/2/index-current.tmp"
        prepared.write_bytes(b"partial")
        with self.assertRaises(WorkError) as context:
            recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        self.assertEqual(context.exception.code, "draft_recovery_conflict")
        (self.storage / "history/2/unknown.json").write_bytes(b"unknown")
        with self.assertRaises(WorkError) as context:
            recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        self.assertEqual(context.exception.code, "draft_recovery_conflict")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)

    def test_recovery_rejects_incomplete_history(self) -> None:
        self.initialize()
        proposed = copy.deepcopy(self.index)
        proposed["revision"] = 2
        proposed["tasks"][0]["status"] = "in_progress"
        with patch("worklib.artifacts.task_draft._write", side_effect=OSError("disk full")):
            with self.assertRaises(WorkError):
                self.save()
        with self.assertRaises(WorkError) as context:
            recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        self.assertEqual(context.exception.code, "draft_recovery_incomplete")

    def test_recovery_rejects_newer_committed_index(self) -> None:
        proposed = self.interrupted_proposal()
        recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        original = copy.deepcopy(self.draft)
        self.draft["revision"] = 2
        self.save()
        with self.assertRaises(WorkError) as context:
            recover_task_planning(self.root, proposed, expected_revision=1, draft=original)
        self.assertEqual(context.exception.code, "draft_revision_conflict")
        self.assertEqual(read_task_planning_index(self.root, "example")["revision"], 3)

    def test_recovery_write_failure_retains_prepared_state(self) -> None:
        proposed = self.interrupted_proposal()
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=OSError("busy")):
            with self.assertRaises(WorkError) as context:
                recover_task_planning(self.root, proposed, expected_revision=1, draft=self.draft)
        self.assertEqual(context.exception.code, "draft_recovery_interrupted")
        self.assertTrue((self.storage / "history/2/index-current.tmp").is_file())
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)


if __name__ == "__main__":
    unittest.main()
