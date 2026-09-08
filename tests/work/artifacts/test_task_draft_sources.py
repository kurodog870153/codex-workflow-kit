from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.task_draft import save_task_planning
from worklib.artifacts.task_draft_sources import check_task_draft_sources
from worklib.contracts.plan import render_plan_contract, validate_plan_file
from worklib.foundation.errors import ExitCode, WorkError
from worklib.hierarchy.selection import build_hierarchy_selection
from worklib.instructions.selection import build_instruction_selection
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.skills.selection import selection_sha256


class TaskDraftSourceTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        work_root = SCRIPT_ROOT.parent
        hierarchy = build_hierarchy_selection({"decision": "general_only", "selections": []}, skill_root=work_root)
        self.plan = {
            "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed",
            "title": "Plan", "summary": "Result",
            "artifacts": {"plan": "outputs/work/plans/example.md", "task": "outputs/work/tasks/example/task.md", "execution": "outputs/work/executions/example"},
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

    def initialize(self):
        save_task_planning(self.root, self.index, expected_revision=0)

    def invoke(self, *, expected_revision=1, task_id="TASK-001"):
        try:
            result = check_task_draft_sources(
                self.root, "example", task_id, expected_revision=expected_revision,
                plan_path=self.plan["artifacts"]["plan"], user_config_root=str(self.root),
                selected_paths=[], reference_names=[],
            )
            return ExitCode.SUCCESS, result
        except WorkError as error:
            return error.exit_code, error.as_dict()

    def test_live_sources_pass_without_writing(self):
        self.initialize()
        before = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        code, result = self.invoke()
        self.assertEqual(code, ExitCode.SUCCESS)
        self.assertEqual(result["schema"], "work-task-draft-source-check/v1")
        self.assertEqual(result["status"], "valid")
        self.assertEqual(before, {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob("*") if p.is_file()})

    def test_changed_plan_content_is_rejected(self):
        self.initialize()
        self.plan["summary"] = "Changed result"
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        code, result = self.invoke()
        self.assertEqual(code, ExitCode.ARTIFACT_INTEGRITY)
        self.assertEqual(result["code"], "draft_source_drift")
        self.assertEqual(result["details"]["fields"], ["plan_sha256"])

    def test_stale_skill_selection_hash_is_rejected(self):
        self.index["source"]["skill_selection_sha256"] = "0" * 64
        self.initialize()
        self.assertEqual(self.invoke()[1]["code"], "draft_source_drift")

    def test_instruction_fingerprint_mismatch_is_rejected(self):
        self.index["tasks"][0]["instructions_sha256"] = "0" * 64
        self.initialize()
        self.assertEqual(self.invoke()[1]["code"], "draft_instruction_drift")

    def test_skill_binding_must_exist_in_plan(self):
        self.index["tasks"][0]["skill_id"] = "unknown"
        self.initialize()
        self.assertEqual(self.invoke()[1]["code"], "draft_skill_not_available")

    def test_stale_revision_and_unknown_task_are_rejected(self):
        self.initialize()
        self.assertEqual(self.invoke(expected_revision=2)[1]["code"], "draft_revision_conflict")
        self.assertEqual(self.invoke(task_id="TASK-002")[1]["code"], "draft_task_not_in_index")

    def test_plan_validation_failure_is_preserved(self):
        self.initialize()
        with patch("worklib.artifacts.task_draft_sources.validate_plan_contract", side_effect=WorkError(ExitCode.ARTIFACT_INTEGRITY, "skill_bundle_drift", "Changed skill.")):
            self.assertEqual(self.invoke()[1]["code"], "skill_bundle_drift")
