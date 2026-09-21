"""Cross-feature orchestration for delegation validation."""
from __future__ import annotations

import re
from pathlib import Path

from ...models.progress import FIELDS
from ...services.delegation import (
    MAIN_MODES,
    artifact_paths,
    delegation_skill_root,
    delegation_validation_result,
    fail,
    nonempty_object,
    nonempty_string,
    parse_delegation_request,
    requirement_id,
    sha256,
    strict_keys,
    text_array,
    validate_delegation_envelope,
)
from ...services.hierarchy.fingerprint import hierarchy_selection_sha256
from ...services.hierarchy.validation import SELECTION_FIELDS
from ...services.instruction.history import stored_selection
from ...services.invocation import parse_invocation
from ...services.progress.validation import validate_progress_contract
from ...services.skill_selection import SKILL_FIELDS, TOP_FIELDS, selection_sha256


def _skills(value):
    selection = strict_keys(value, location="skill_selection", required=set(TOP_FIELDS))
    if selection["schema"] != "work-skill-selection/v1" or not isinstance(selection["skills"], list):
        fail("Supply a Work skill selection snapshot.")
    skills = selection["skills"]
    if selection["decision"] != ("external_skills" if skills else "base_only"):
        fail("Skill selection decision and cardinality disagree.")
    ids = []
    for skill in skills:
        strict_keys(skill, location="skill", required=set(SKILL_FIELDS))
        ids.append(nonempty_string(skill["id"], location="skill.id"))
        for field in ("summary_sha256", "bundle_sha256"):
            sha256(skill[field], location=field)
        support = strict_keys(skill["mode_support"], location="mode_support", required={"plan", "task", "execute"})
        if any(item not in ("declared", "inferred", "unsupported") for item in support.values()):
            fail("Invalid skill mode support.")
    if len(ids) != len(set(ids)) or selection["selection_sha256"] != selection_sha256(selection["decision"], skills):
        fail("Skill snapshot identities or selection fingerprint disagree.")
    return selection


def _hierarchy(value):
    hierarchy = strict_keys(value, location="hierarchy_selection", required=set(SELECTION_FIELDS))
    if hierarchy["schema"] != "work-hierarchy-selection/v1":
        fail("Supply the confirmed hierarchy snapshot.")
    text_array(hierarchy["selected_paths"], location="selected_paths")
    if not isinstance(hierarchy["entries"], list):
        fail("Hierarchy entries must be an array.")
    sha256(hierarchy["catalog_sha256"], location="catalog_sha256")
    if hierarchy["selection_sha256"] != hierarchy_selection_sha256(
        hierarchy["decision"], hierarchy["selected_paths"], hierarchy["entries"], hierarchy["catalog_sha256"]
    ):
        fail("The stored hierarchy fingerprint disagrees with its fields.")


def _selections(context):
    _hierarchy(context["hierarchy_selection"])
    stored_selection(context["work_instruction_selection"])
    _skills(context["skill_selection"])


def validate_delegation(value, *, role: str, sender: str, project_root: Path, skill_root: Path):
    envelope, request, mode, context, resume = validate_delegation_envelope(
        value, role=role, sender=sender, project_root=project_root, skill_root=skill_root,
    )
    if resume:
        if role not in {"plan", "task-coordinator"}:
            fail("Only Plan and Task may restore discussion.")
        strict_keys(context, location="resume_context", required={"saved_progress"})
        progress = validate_progress_contract(context["saved_progress"])
        entry = parse_invocation(
            f"$work {mode} -- {request}".encode(), source="delegation request"
        ).to_canonical_dict()["entry"]
        if entry != {"kind": "progress_resume", "requirement_id": progress["requirement_id"]} or progress["mode"] != mode:
            fail("Resume request, mode and saved discussion identity disagree.")
    elif role in MAIN_MODES:
        fields = {"hierarchy_selection", "work_instruction_selection", "skill_selection"}
        fields |= {"source_plan"} if role == "task-coordinator" else ({"target_task", "hierarchy_selection_sha256", "execute_skill_selection"} if role == "execute" else set())
        strict_keys(context, location="workflow_context", required=fields)
        _selections(context)
        if role == "task-coordinator":
            plan = nonempty_object(context["source_plan"], location="source_plan")
            if plan.get("schema") != "work-plan/v1" or any(plan.get(key) != context[key] for key in ("hierarchy_selection", "skill_selection")):
                fail("Source Plan selections disagree with the envelope.")
        if role == "execute":
            task = nonempty_object(context["target_task"], location="target_task")
            if not isinstance(task.get("id"), str) or not re.fullmatch(r"TASK-[0-9]{3}", task["id"]) or "skill_id" not in task:
                fail("Execute requires one explicit target TASK row.")
            selection = _skills(context["execute_skill_selection"])
            expected = [skill for skill in context["skill_selection"]["skills"] if skill["id"] == task["skill_id"]]
            if (selection["skills"] != expected or len(expected) != (0 if task["skill_id"] is None else 1)
                or context["hierarchy_selection_sha256"] != context["hierarchy_selection"]["selection_sha256"]):
                fail("Execute must retain the target's exact skill and hierarchy identity.")
            if any(skill["mode_support"]["execute"] == "unsupported" or skill["dependency_status"] != "available" for skill in expected):
                fail("Execute requires available, supported skills.")
    elif role == "task-skill":
        strict_keys(context, location="task_skill_context", required={"task_boundary", "skill_snapshot", "source_plan",
            "work_instruction_selection", "repository_evidence", "saved_discussion"})
        boundary = nonempty_object(context["task_boundary"], location="task_boundary")
        if not isinstance(boundary.get("id"), str) or not re.fullmatch(r"TASK-[0-9]{3}", boundary["id"]):
            fail("Task refinement requires one identified TASK boundary.")
        for field in ("title", "goal"):
            nonempty_string(boundary.get(field), location="task_boundary." + field)
        skill = strict_keys(context["skill_snapshot"], location="skill_snapshot", required=set(SKILL_FIELDS))
        _skills({"schema": "work-skill-selection/v1", "decision": "external_skills", "skills": [skill],
                 "selection_sha256": selection_sha256("external_skills", [skill])})
        plan = nonempty_object(context["source_plan"], location="source_plan")
        selection = _skills(plan.get("skill_selection"))
        if boundary.get("skill_id") != skill["id"] or skill not in selection["skills"] or skill["mode_support"]["task"] == "unsupported" or skill["dependency_status"] != "available":
            fail("Task refinement requires exactly its Plan-confirmed executable skill.")
        stored_selection(context["work_instruction_selection"])
        text_array(context["repository_evidence"], location="repository_evidence")
        text_array(context["saved_discussion"], location="saved_discussion")
    else:
        common = {"requirement_id", "continuation_point"}
        fields = {"content", "expected_revision"} if role == "progress-saver" else {
            "artifacts", "confirmed_request", "decisions", "affected_task_ids", "hierarchy_selection", "skill_selection", "repository_evidence"}
        strict_keys(context, location="maintenance_context", required=common | fields,
                    optional={"save_approval"} if role == "progress-saver" else set())
        requirement = requirement_id(context["requirement_id"])
        nonempty_string(context["continuation_point"], location="continuation_point")
        if role == "progress-saver":
            revision = context["expected_revision"]
            if type(revision) is not int or revision < 0:
                fail("Expected saved revision must be a nonnegative integer.")
            content = strict_keys(context["content"], location="content", required=set(FIELDS) - {
                "schema", "requirement_id", "mode", "revision", "status"})
            validate_progress_contract({**content, "schema": "work-discussion-progress/v1", "requirement_id": requirement,
                "mode": mode, "revision": revision + 1, "status": "discussion_only"})
            if "save_approval" in context:
                sha256(context["save_approval"], location="save_approval")
        else:
            artifact_paths(project_root, requirement, context["artifacts"])
            nonempty_object(context["confirmed_request"], location="confirmed_request")
            _hierarchy(context["hierarchy_selection"])
            _skills(context["skill_selection"])
            if not isinstance(context["decisions"], list) or not context["decisions"]:
                fail("Artifact revision requires retained confirmed decisions.")
            for decision in context["decisions"]:
                if isinstance(decision, dict):
                    nonempty_object(decision, location="decision")
                else:
                    nonempty_string(decision, location="decision")
            text_array(context["affected_task_ids"], location="affected_task_ids", task_ids=True)
            text_array(context["repository_evidence"], location="repository_evidence")
    return delegation_validation_result(role=role, mode=mode, resume=resume)


def validate_delegation_request(raw: bytes, *, source: str, role: str, sender: str, project_root: Path):
    return validate_delegation(
        parse_delegation_request(raw, source=source),
        role=role,
        sender=sender,
        project_root=project_root,
        skill_root=delegation_skill_root(),
    )
