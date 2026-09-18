from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_task as fixtures
from contracts.test_progress import discussion
from worklib.business_services.delegation import validate_delegation
from worklib.services.delegation import MARKERS, validate_delegation_envelope
from worklib.models.common.errors import WorkError
from worklib.business_services.instruction import build_instruction_selection
from worklib.services.skill_selection import SKILL_FIELDS, selection_sha256


class DelegationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.TaskInstructionContractTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.project_root
        self.plan = json.loads((self.root / self.fixture.artifacts["plan"]).read_bytes())

    def envelope(self, role="plan"):
        mode = {"plan": "plan", "execute": "execute", "task-coordinator": "task", "task-skill": "task",
                "artifact-editor": "execute", "progress-saver": "task"}[role]
        context = {key: copy.deepcopy(self.plan[key]) for key in ("hierarchy_selection", "skill_selection")}
        context["work_instruction_selection"] = build_instruction_selection(skill_root=fixtures.SKILL_ROOT, mode=mode, selected_paths=[])
        if role == "task-coordinator":
            context["source_plan"] = copy.deepcopy(self.plan)
        elif role == "execute":
            context.update(target_task=copy.deepcopy(self.fixture.contract["tasks"][0]),
                hierarchy_selection_sha256=context["hierarchy_selection"]["selection_sha256"],
                execute_skill_selection=copy.deepcopy(context["skill_selection"]))
        elif role == "artifact-editor":
            context.pop("work_instruction_selection")
            context.update(requirement_id="example", artifacts=self.fixture.artifacts, confirmed_request={"reason": "Reviewed"},
                decisions=["Confirmed revision"], affected_task_ids=["TASK-001"], repository_evidence=[], continuation_point="Return to Execute")
        elif role == "progress-saver":
            content = discussion()
            for key in ("schema", "requirement_id", "mode", "revision", "status"):
                del content[key]
            context = dict(requirement_id="example", content=content, expected_revision=0, continuation_point="Continue discussion")
        elif role == "task-skill":
            skill = {key: "example" for key in SKILL_FIELDS}
            skill.update(mode_support={key: "inferred" for key in ("plan", "task", "execute")},
                dependency_status="available", summary_sha256="a" * 64, bundle_sha256="b" * 64, allow_implicit_invocation=True)
            plan = copy.deepcopy(self.plan)
            plan["skill_selection"] = {"schema": "work-skill-selection/v1", "decision": "external_skills", "skills": [skill],
                "selection_sha256": selection_sha256("external_skills", [skill])}
            context = dict(task_boundary={"id": "TASK-001", "title": "Task", "goal": "Reviewed scope", "skill_id": skill["id"]},
                skill_snapshot=skill, source_plan=plan, work_instruction_selection=context["work_instruction_selection"],
                repository_evidence=[], saved_discussion=[])
        return dict(schema="work-delegation-envelope/v1", marker=MARKERS[role], skill="$work", role=role,
            sender="task-coordinator" if role == "task-skill" else "parent", mode=mode, request="Confirmed role request",
            project_root=str(self.root), skill_root=str(fixtures.SKILL_ROOT.resolve()), context=context)

    def validate(self, value, **options):
        return validate_delegation(value, role=value["role"], sender=value["sender"], project_root=self.root,
                                   skill_root=fixtures.SKILL_ROOT, **options)

    def test_all_roles_validate_without_mutation_or_authority(self):
        for role in MARKERS:
            value = self.envelope(role)
            before = copy.deepcopy(value)
            with self.subTest(role=role):
                result = self.validate(value)
                self.assertEqual(result["status"], "valid")
                self.assertFalse(result["grants_authorization"])
                self.assertEqual(result["source_validation"], "not_checked")
                self.assertEqual(value, before)

    def test_role_sender_marker_roots_and_extra_authority_are_rejected(self):
        for key, changed in (("marker", "WORK_PROGRESS_SAVE_V1"), ("sender", "execute"), ("mode", "execute"),
                             ("project_root", "."), ("skill", "$other"), ("request", ""), ("authorized", True)):
            value = self.envelope()
            value[key] = changed
            with self.subTest(key=key), self.assertRaises(WorkError):
                self.validate(value)
        value = self.envelope()
        with self.assertRaises(WorkError):
            validate_delegation(value, role="execute", sender="parent", project_root=self.root, skill_root=fixtures.SKILL_ROOT)

    def test_envelope_service_validates_without_cross_feature_context(self):
        value = self.envelope()
        envelope, request, mode, context, resume = validate_delegation_envelope(
            value,
            role="plan",
            sender="parent",
            project_root=self.root,
            skill_root=fixtures.SKILL_ROOT,
        )
        self.assertIs(envelope, value)
        self.assertEqual(request, "Confirmed role request")
        self.assertEqual(mode, "plan")
        self.assertIs(context, value["context"])
        self.assertFalse(resume)

    def test_missing_role_context_is_rejected(self):
        for role in MARKERS:
            value = self.envelope(role)
            del value["context"][next(iter(value["context"]))]
            with self.subTest(role=role), self.assertRaises(WorkError):
                self.validate(value)

    def test_resume_preserves_unavailable_sources_and_checks_identity(self):
        for role, mode in (("plan", "plan"), ("task-coordinator", "task")):
            value = self.envelope(role)
            progress = discussion()
            progress.update(mode=mode, current_task_id=None)
            value.update(request="resume example", context={"saved_progress": progress})
            self.assertEqual(self.validate(value)["scope"], "discussion_restoration")
            value["request"] = "resume other"
            with self.assertRaises(WorkError):
                self.validate(value)

    def test_resume_cannot_hide_execute_or_mix_current_selections(self):
        for role in ("execute", "progress-saver", "plan"):
            value = self.envelope(role)
            value["context"]["saved_progress"] = discussion()
            with self.subTest(role=role), self.assertRaises(WorkError):
                self.validate(value)

    def test_skill_and_hierarchy_bindings_cannot_change(self):
        value = self.envelope("execute")
        value["context"]["target_task"]["skill_id"] = "unconfirmed"
        with self.assertRaises(WorkError):
            self.validate(value)
        value = self.envelope("task-skill")
        value["context"]["task_boundary"]["skill_id"] = "another"
        with self.assertRaises(WorkError):
            self.validate(value)
        value = self.envelope("task-coordinator")
        value["context"]["source_plan"]["hierarchy_selection"]["selection_sha256"] = "0" * 64
        with self.assertRaises(WorkError):
            self.validate(value)

    def test_progress_saver_rejects_bool_revision_and_foreign_content(self):
        value = self.envelope("progress-saver")
        value["context"]["expected_revision"] = True
        with self.assertRaises(WorkError):
            self.validate(value)
        value = self.envelope("progress-saver")
        value["context"]["content"]["mode"] = "execute"
        with self.assertRaises(WorkError):
            self.validate(value)
