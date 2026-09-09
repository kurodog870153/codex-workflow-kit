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

from worklib.artifacts.task_draft import read_task_draft, read_task_planning_index, save_task_planning
from worklib.artifacts.task_draft_request import save_task_draft_request
from worklib.cli import main
from worklib.contracts.plan import render_plan_contract, validate_plan_file
from worklib.foundation.errors import ExitCode, WorkError
from worklib.hierarchy.selection import build_hierarchy_selection
from worklib.instructions.selection import build_instruction_selection
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.skills.selection import selection_sha256


class TaskDraftRequestTests(unittest.TestCase):
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
            "current_task_id": "TASK-001", "retired_task_ids": ["TASK-003"],
            "source": {key: checked[key] for key in ("plan_sha256", "hierarchy_selection_sha256", "skill_selection_sha256")},
            "tasks": [{"id": task_id, "title": "Task", "goal": "Result", "scope": ["Source"], "skill_id": None,
                       "dependencies": [], "status": "planned", "boundary_revision": 1, "instructions_sha256": selection["instructions_sha256"]}
                      for task_id in ("TASK-001", "TASK-002")],
        }
        save_task_planning(self.root, self.index, expected_revision=0)
        self.storage = self.root / "outputs/work/tasks/example/drafts"
        self.request = {
            "status": "in_progress", "notes": ["保留具體討論內容"], "confirmed_decisions": [],
            "tentative": [], "open_questions": ["Which test?"], "next_discussion_point": "Confirm test.",
        }
        self.options = dict(expected_revision=1, plan_path=self.plan["artifacts"]["plan"],
                            user_config_root=str(self.root), selected_paths=[], reference_names=[])

    def save(self, *, task_id="TASK-001", recover=False):
        return save_task_draft_request(self.root, "example", task_id, self.request, recover=recover, **self.options)

    def snapshot(self):
        return {path.relative_to(self.storage): path.read_bytes() for path in self.storage.rglob("*") if path.is_file()}

    def assert_rejected(self, code, **kwargs):
        before = self.snapshot()
        with self.assertRaises(WorkError) as context:
            self.save(**kwargs)
        self.assertEqual(context.exception.code, code)
        self.assertEqual(self.snapshot(), before)

    def interrupt(self):
        with patch("worklib.artifacts.task_draft.os.replace", side_effect=OSError("busy")):
            with self.assertRaises(WorkError) as context:
                self.save()
        self.assertEqual(context.exception.code, "draft_save_interrupted")
        self.assertEqual(read_task_planning_index(self.root, "example"), self.index)

    def test_derives_versions_and_preserves_other_saved_task_and_history(self):
        original = copy.deepcopy(self.request)
        self.save(task_id="TASK-002")
        previous = read_task_planning_index(self.root, "example")
        historical = self.snapshot()
        self.options["expected_revision"] = 2
        self.save()
        self.options["expected_revision"] = 3
        self.request["notes"] = ["Updated discussion"]
        self.save()
        current = read_task_planning_index(self.root, "example")
        draft = read_task_draft(self.root, "example", "TASK-001")
        self.assertEqual((current["revision"], draft["revision"], draft["boundary_revision"]), (4, 2, 1))
        self.assertEqual(current["tasks"][1], previous["tasks"][1])
        self.assertEqual(current["source"], self.index["source"])
        self.assertEqual(current["retired_task_ids"], ["TASK-003"])
        self.assertEqual(current["current_task_id"], "TASK-001")
        for path, raw in historical.items():
            if path.parts[0] == "history":
                self.assertEqual((self.storage / path).read_bytes(), raw)
        self.assertEqual(read_task_draft(self.root, "example", "TASK-002")["notes"], original["notes"])
        self.assertEqual(draft["notes"], self.request["notes"])
        self.assertNotIn("revision", self.request)
        self.assertFalse((self.root / self.plan["artifacts"]["task"]).exists())
        self.assertFalse((self.root / self.plan["artifacts"]["execution"]).exists())

    def test_first_save_records_selection_and_later_save_restores_it(self):
        historical = (self.storage / "history/1/index.json").read_bytes()
        self.save()
        index = read_task_planning_index(self.root, "example")
        self.assertEqual(index["tasks"][0]["instruction_selection"], {"selected_paths": [], "references": []})
        self.assertNotIn("instruction_selection", index["tasks"][1])
        self.options.update(expected_revision=2, selected_paths=None, reference_names=None)
        self.save()
        self.assertEqual(self.save(recover=True)["status"], "already_completed")
        self.assertEqual((self.storage / "history/1/index.json").read_bytes(), historical)
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001")["boundary_revision"], 1)

    def test_missing_or_changed_selection_does_not_write(self):
        self.options.update(selected_paths=None, reference_names=None)
        self.assert_rejected("draft_selection_required")
        self.options.update(selected_paths=[], reference_names=[])
        self.save()
        self.options.update(expected_revision=2, reference_names=["task.general.task-records"])
        self.assert_rejected("draft_selection_mismatch")

    def test_cli_uses_saved_selection_without_flags(self):
        self.save()
        output, error = io.StringIO(), io.StringIO()
        code = main([
            "--project-root", str(self.root), "task", "draft-save-request", "--stdin",
            "--requirement-id", "example", "--task-id", "TASK-001", "--expected-revision", "2",
            "--plan-path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root),
        ], stdin=io.StringIO(json.dumps(self.request)), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output.getvalue())["revision"], 3)

    def test_rejects_storage_metadata_and_boundary_fields(self):
        for field in ("index", "revision", "source", "instructions_sha256", "task_id", "scope"):
            with self.subTest(field=field):
                self.request[field] = "caller supplied"
                before = self.snapshot()
                with self.assertRaises(WorkError):
                    self.save()
                self.assertEqual(self.snapshot(), before)
                del self.request[field]

    def test_stale_revision_unknown_task_and_invalid_revision_do_not_write(self):
        self.assert_rejected("draft_task_not_in_index", task_id="TASK-004")
        for revision in (0, -1, True):
            self.options["expected_revision"] = revision
            self.assert_rejected("invalid_expected_revision")
        self.options["expected_revision"] = 2
        self.assert_rejected("draft_revision_conflict")

    def test_changed_plan_and_instruction_selection_do_not_write(self):
        self.plan["summary"] = "Changed result"
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        self.assert_rejected("draft_source_drift")
        self.plan["summary"] = "Result"
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        self.options["reference_names"] = ["task.general.task-records"]
        self.assert_rejected("draft_instruction_drift")

    def test_skill_validation_failure_is_preserved_without_writing(self):
        with patch("worklib.artifacts.task_draft_sources.validate_plan_contract", side_effect=WorkError(ExitCode.ARTIFACT_INTEGRITY, "skill_bundle_drift", "Changed skill.")):
            self.assert_rejected("skill_bundle_drift")

    def test_refined_content_and_candidate_use_existing_contract(self):
        self.request["status"] = "refined"
        self.assert_rejected("unfinished_refined_draft")
        self.request["open_questions"] = []
        self.request["next_discussion_point"] = None
        self.request["task_candidate"] = {"id": "TASK-002"}
        self.assert_rejected("task_candidate_boundary_mismatch")
        entry = self.index["tasks"][0]
        self.request["task_candidate"] = {
            **{field: entry[field] for field in ("id", "title", "goal", "skill_id", "dependencies")},
            "instruction_selection": {"instructions_sha256": entry["instructions_sha256"]},
        }
        self.save()
        draft = read_task_draft(self.root, "example", "TASK-001")
        self.assertEqual(draft["task_candidate"], self.request["task_candidate"])
        self.assertEqual(draft["status"], "refined")

    def test_request_replaces_discussion_and_does_not_retain_omitted_candidate(self):
        self.request["task_candidate"] = {"id": "TASK-001"}
        self.save()
        self.options["expected_revision"] = 2
        del self.request["task_candidate"]
        self.request["notes"] = ["Replacement discussion"]
        self.save()
        draft = read_task_draft(self.root, "example", "TASK-001")
        self.assertNotIn("task_candidate", draft)
        self.assertEqual(draft["notes"], self.request["notes"])

    def test_interrupted_save_recovers_from_identical_small_request(self):
        self.interrupt()
        result = self.save(recover=True)
        self.assertEqual(result["status"], "recovered")
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001")["revision"], 1)
        self.assertEqual(self.save(recover=True)["status"], "already_completed")

    def test_recovery_rejects_changed_request_and_sources(self):
        self.interrupt()
        self.request["notes"] = ["Different content"]
        self.assert_rejected("draft_recovery_conflict", recover=True)
        self.request["notes"] = ["保留具體討論內容"]
        self.plan["summary"] = "Changed result"
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        self.assert_rejected("draft_source_drift", recover=True)

    def test_partial_save_cannot_be_reconstructed_by_recovery(self):
        with patch("worklib.artifacts.task_draft._write", side_effect=OSError("disk full")):
            with self.assertRaises(WorkError):
                self.save()
        self.assert_rejected("draft_recovery_incomplete", recover=True)

    def test_recovery_rejects_newer_progress(self):
        self.save()
        self.options["expected_revision"] = 2
        self.save()
        self.options["expected_revision"] = 1
        self.assert_rejected("draft_revision_conflict", recover=True)

    def test_successful_save_can_be_recognized_without_incrementing_again(self):
        self.save()
        before = self.snapshot()
        self.assertEqual(self.save(recover=True)["status"], "already_completed")
        self.assertEqual(self.snapshot(), before)

    def test_cli_saves_small_payload_end_to_end(self):
        output, error = io.StringIO(), io.StringIO()
        code = main([
            "--project-root", str(self.root), "task", "draft-save-request", "--stdin",
            "--requirement-id", "example", "--task-id", "TASK-001", "--expected-revision", "1",
            "--plan-path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root), "--general-only",
        ], stdin=io.StringIO(json.dumps(self.request)), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(output.getvalue())["revision"], 2)
        self.assertEqual(read_task_draft(self.root, "example", "TASK-001")["notes"], self.request["notes"])


if __name__ == "__main__":
    unittest.main()
