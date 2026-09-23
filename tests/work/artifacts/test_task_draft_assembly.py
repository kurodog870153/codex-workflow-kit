from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.services.task.draft.storage import save_task_planning, read_task_planning_index
from worklib.orchestration.task import assemble_task_drafts, create_task_from_drafts
from worklib.business_services.plan import render_plan_contract, validate_plan_contract
from worklib.business_services.task import load_task_collection
from worklib.models.common.errors import WorkError
from worklib.business_services.hierarchy import build_hierarchy_selection
from worklib.business_services.instruction import build_instruction_selection
from worklib.business_services.instruction import build_work_instruction_selection
from worklib.services.skill_selection import selection_sha256


class TaskDraftAssemblyTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        work_root = SCRIPT_ROOT.parent
        hierarchy = build_hierarchy_selection({"decision": "general_only", "selections": []}, skill_root=work_root)
        self.plan = {
            "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed", "title": "Plan", "summary": "Result",
            "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
            "hierarchy_selection": hierarchy,
            "work_instruction_selection": build_work_instruction_selection(skill_root=work_root, mode="plan", selected_paths=[]),
            "skill_selection": {"schema": "work-skill-selection/v1", "decision": "base_only", "skills": [], "selection_sha256": selection_sha256("base_only", [])},
            "goals": [{"id": "GOAL-001", "statement": "Result"}],
            "scope": [{"id": "SCOPE-001", "kind": "in_scope", "statement": "Source", "goal_ids": ["GOAL-001"]}],
            "deliverables": [{"id": "DELIVERABLE-001", "statement": "Result", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}],
            "acceptance_criteria": [{"id": "ACCEPTANCE-001", "statement": "Observable", "deliverable_ids": ["DELIVERABLE-001"]}],
        }
        path = self.root / self.plan["artifacts"]["plan"]
        path.parent.mkdir(parents=True)
        path.write_bytes(render_plan_contract(self.plan))
        validation = validate_plan_contract(
            path.read_bytes(), source=str(path),
            actual_plan_path=self.plan["artifacts"]["plan"],
            project_root=self.root, user_config_root=str(self.root),
            _allow_task_index=True,
        )
        selection = build_instruction_selection(skill_root=work_root, mode="task", selected_paths=[], reference_names=["task.general.task-records"])
        source = {key: validation[key] for key in ("plan_sha256", "hierarchy_selection_sha256", "skill_selection_sha256")}
        index = {"schema": "work-task-planning-index/v1", "requirement_id": "example", "revision": 1, "current_task_id": "TASK-001", "source": source,
                 "tasks": [{"id": "TASK-001", "title": "Task", "goal": "Result", "scope": ["Source"], "skill_id": None, "dependencies": [], "status": "planned", "boundary_revision": 1, "instructions_sha256": selection["instructions_sha256"]}]}
        save_task_planning(self.root, index, expected_revision=0)
        candidate = {"id": "TASK-001", "title": "Task", "goal": "Result", "skill_id": None, "instruction_selection": selection,
                     "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
                     "steps": [{"id": "STEP-001", "action": "Review result.", "references": ["VAL-001"]}],
                     "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "User", "criteria": "Result is observable.", "acceptance_ids": ["ACCEPTANCE-001"]}]}
        self.draft = {"schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001", "revision": 1, "boundary_revision": 1,
                      "source": source, "instructions_sha256": selection["instructions_sha256"], "status": "refined", "notes": [], "confirmed_decisions": [],
                      "tentative": [], "open_questions": [], "next_discussion_point": None, "task_candidate": candidate}
        self.metadata = {"title": "Formal TASK", "summary": "Result"}
        self.options = {"expected_revision": 2, "plan_path": self.plan["artifacts"]["plan"], "user_config_root": str(self.root)}

    def save(self):
        index = read_task_planning_index(self.root, "example")
        expected = index["revision"]
        index["revision"] += 1
        index["tasks"][0]["status"] = self.draft["status"]
        save_task_planning(self.root, index, expected_revision=expected, draft=self.draft)

    def assemble(self):
        return assemble_task_drafts(self.root, "example", self.metadata, **self.options)

    def test_assembly_is_read_only_and_formal_creation_matches_hash(self):
        self.save()
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        assembled = self.assemble()
        self.assertEqual(before, {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()})
        self.assertEqual(assembled["contract"]["tasks"], [self.draft["task_candidate"]])
        self.assertIn("task_collection_sha256", assembled)
        result = create_task_from_drafts(self.root, "example", self.metadata, approved_sha256=assembled["approval_sha256"], **self.options)
        self.assertEqual(result["status"], "created")
        formal = load_task_collection(self.root, str(self.root), self.plan["artifacts"]["task"])
        self.assertEqual(formal["task_collection_sha256"], assembled["task_collection_sha256"])
        self.assertTrue((self.root / self.plan["artifacts"]["execution"] / "index.json").exists())

    def test_changed_metadata_rejects_approval_before_any_formal_write(self):
        self.save()
        approved = self.assemble()["approval_sha256"]
        self.metadata["summary"] = "Changed summary"
        with self.assertRaises(WorkError) as context:
            create_task_from_drafts(self.root, "example", self.metadata, approved_sha256=approved, **self.options)
        self.assertEqual(context.exception.code, "draft_approval_mismatch")
        self.assertFalse((self.root / self.plan["artifacts"]["task"]).exists())
        self.assertFalse((self.root / self.plan["artifacts"]["execution"]).exists())

    def test_new_discussion_revision_invalidates_approval_even_with_same_contract(self):
        self.save()
        first = self.assemble()
        self.draft["revision"] = 2
        self.draft["notes"] = ["More discussion evidence"]
        self.save()
        with self.assertRaises(WorkError) as context:
            self.assemble()
        self.assertEqual(context.exception.code, "draft_revision_conflict")
        self.options["expected_revision"] = 3
        second = self.assemble()
        self.assertEqual(
            {
                key: first[key]
                for key in (
                    "task_collection_sha256",
                    "task_index_sha256",
                    "task_item_sha256",
                )
            },
            {
                key: second[key]
                for key in (
                    "task_collection_sha256",
                    "task_index_sha256",
                    "task_item_sha256",
                )
            },
        )
        self.assertNotEqual(first["approval_sha256"], second["approval_sha256"])

    def test_missing_candidate_and_incomplete_discussion_are_rejected(self):
        self.draft.pop("task_candidate")
        self.save()
        with self.assertRaises(WorkError) as context:
            self.assemble()
        self.assertEqual(context.exception.code, "task_candidate_required")
        self.draft.update(revision=2, status="needs_review", next_discussion_point="Review change")
        self.save()
        self.options["expected_revision"] = 3
        with self.assertRaises(WorkError) as context:
            self.assemble()
        self.assertEqual(context.exception.code, "draft_not_refined")

    def test_full_contract_validation_rejects_missing_steps(self):
        self.draft["task_candidate"].pop("steps")
        self.save()
        with self.assertRaises(WorkError) as context:
            self.assemble()
        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["location"], "contract")
        self.assertEqual(context.exception.details["missing"], ["steps"])

    def test_candidate_cannot_change_confirmed_boundary(self):
        self.draft["task_candidate"]["goal"] = "Different goal"
        with self.assertRaises(WorkError) as context:
            self.save()
        self.assertEqual(context.exception.code, "task_candidate_boundary_mismatch")

    def test_source_drift_rejects_assembly(self):
        self.save()
        self.plan["summary"] = "Changed Plan"
        (self.root / self.plan["artifacts"]["plan"]).write_bytes(render_plan_contract(self.plan))
        with self.assertRaises(WorkError) as context:
            self.assemble()
        self.assertEqual(context.exception.code, "draft_source_drift")
