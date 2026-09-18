"""Internal envelope structure and role boundaries, without source attestation."""
from __future__ import annotations

import re
from pathlib import Path

from pydantic import ValidationError

from ..contracts.delegation import (
    DelegationEnvelopeContract, DelegationValidationContract,
)
from ..contracts.validation import nonempty_string, sha256, strict_keys
from ..contracts.progress import FIELDS, validate_progress_contract
from ..models.common.errors import ExitCode, WorkError
from ..services.invocation import parse_invocation
from ..foundation.paths import validate_artifact_paths, validate_requirement_id
from .instruction_history import stored_selection
from ..services.skill_selection import SKILL_FIELDS, TOP_FIELDS, selection_sha256
from ..services.hierarchy_selection import SELECTION_FIELDS, hierarchy_selection_sha256


ROLES = ("plan", "task-coordinator", "execute", "task-skill", "artifact-editor", "progress-saver")
MAIN_MODES = {"plan": "plan", "task-coordinator": "task", "execute": "execute"}
MARKERS = {**{role: "WORK_DELEGATION_V1" for role in MAIN_MODES},
    "task-skill": "WORK_TASK_SKILL_V1", "artifact-editor": "WORK_ARTIFACT_EDIT_V1",
    "progress-saver": "WORK_PROGRESS_SAVE_V1"}


def _fail(message):
    raise WorkError(ExitCode.CONTRACT, "delegation_boundary_mismatch", message)


def _object(value, location):
    if not isinstance(value, dict) or not value:
        _fail(location + " must be a nonempty object.")
    return value


def _texts(value, location, *, task_ids=False):
    if not isinstance(value, list):
        _fail(location + " must be an array.")
    for item in value:
        nonempty_string(item, location=location)
        if task_ids and not re.fullmatch(r"TASK-[0-9]{3}", item):
            _fail("Affected TASK IDs must use TASK-nnn.")
    if task_ids and len(value) != len(set(value)):
        _fail("Affected TASK IDs must be unique.")


def _skills(value):
    selection = strict_keys(value, location="skill_selection", required=set(TOP_FIELDS))
    if selection["schema"] != "work-skill-selection/v1" or not isinstance(selection["skills"], list):
        _fail("Supply a Work skill selection snapshot.")
    skills = selection["skills"]
    if selection["decision"] != ("external_skills" if skills else "base_only"):
        _fail("Skill selection decision and cardinality disagree.")
    ids = []
    for skill in skills:
        strict_keys(skill, location="skill", required=set(SKILL_FIELDS))
        ids.append(nonempty_string(skill["id"], location="skill.id"))
        for field in ("summary_sha256", "bundle_sha256"):
            sha256(skill[field], location=field)
        support = strict_keys(skill["mode_support"], location="mode_support", required={"plan", "task", "execute"})
        if any(value not in ("declared", "inferred", "unsupported") for value in support.values()):
            _fail("Invalid skill mode support.")
    if len(ids) != len(set(ids)) or selection["selection_sha256"] != selection_sha256(selection["decision"], skills):
        _fail("Skill snapshot identities or selection fingerprint disagree.")
    return selection


def _hierarchy(value):
    hierarchy = strict_keys(value, location="hierarchy_selection", required=set(SELECTION_FIELDS))
    if hierarchy["schema"] != "work-hierarchy-selection/v1":
        _fail("Supply the confirmed hierarchy snapshot.")
    _texts(hierarchy["selected_paths"], "selected_paths")
    if not isinstance(hierarchy["entries"], list):
        _fail("Hierarchy entries must be an array.")
    sha256(hierarchy["catalog_sha256"], location="catalog_sha256")
    if hierarchy["selection_sha256"] != hierarchy_selection_sha256(hierarchy["decision"],
        hierarchy["selected_paths"], hierarchy["entries"], hierarchy["catalog_sha256"]):
        _fail("The stored hierarchy fingerprint disagrees with its fields.")


def _selections(context):
    _hierarchy(context["hierarchy_selection"])
    stored_selection(context["work_instruction_selection"])
    _skills(context["skill_selection"])


def validate_delegation(value, *, role: str, sender: str, project_root: Path, skill_root: Path):
    try:
        DelegationEnvelopeContract.model_validate(value)
    except ValidationError as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "delegation_boundary_mismatch",
            "The delegation envelope structure is invalid.",
        ) from error
    if role not in ROLES or sender != ("task-coordinator" if role == "task-skill" else "parent"):
        _fail("The expected sender cannot delegate to this role.")
    envelope = strict_keys(value, location="delegation", required={
        "schema", "marker", "skill", "role", "sender", "mode", "project_root", "skill_root", "request", "context",
    })
    if (envelope["schema"] != "work-delegation-envelope/v1" or envelope["marker"] != MARKERS[role]
        or envelope["skill"] != "$work" or envelope["role"] != role or envelope["sender"] != sender):
        _fail("Envelope marker, skill, role or sender differs from the receiving context.")
    for field, expected in (("project_root", project_root), ("skill_root", skill_root)):
        declared = Path(nonempty_string(envelope[field], location=field))
        if not declared.is_absolute() or str(declared) != str(declared.resolve()) or declared.resolve() != expected.resolve():
            _fail("Envelope roots must match the resolved receiving roots.")
    request = nonempty_string(envelope["request"], location="request")
    mode = envelope["mode"]
    allowed = (MAIN_MODES[role],) if role in MAIN_MODES else (("task",) if role == "task-skill" else (
        ("plan", "task") if role == "progress-saver" else ("plan", "task", "execute")))
    if mode not in allowed:
        _fail("The role does not accept this mode.")
    context = _object(envelope["context"], "context")
    resume = "saved_progress" in context
    if resume:
        if role not in {"plan", "task-coordinator"}:
            _fail("Only Plan and Task may restore discussion.")
        strict_keys(context, location="resume_context", required={"saved_progress"})
        progress = validate_progress_contract(context["saved_progress"])
        entry = parse_invocation(
            f"$work {mode} -- {request}".encode(), source="delegation request"
        ).to_canonical_dict()["entry"]
        if entry != {"kind": "progress_resume", "requirement_id": progress["requirement_id"]} or progress["mode"] != mode:
            _fail("Resume request, mode and saved discussion identity disagree.")
    elif role in MAIN_MODES:
        fields = {"hierarchy_selection", "work_instruction_selection", "skill_selection"}
        fields |= {"source_plan"} if role == "task-coordinator" else ({"target_task", "hierarchy_selection_sha256", "execute_skill_selection"} if role == "execute" else set())
        strict_keys(context, location="workflow_context", required=fields)
        _selections(context)
        if role == "task-coordinator":
            plan = _object(context["source_plan"], "source_plan")
            if plan.get("schema") != "work-plan/v1" or any(plan.get(key) != context[key] for key in ("hierarchy_selection", "skill_selection")):
                _fail("Source Plan selections disagree with the envelope.")
        if role == "execute":
            task = _object(context["target_task"], "target_task")
            if not isinstance(task.get("id"), str) or not re.fullmatch(r"TASK-[0-9]{3}", task["id"]) or "skill_id" not in task:
                _fail("Execute requires one explicit target TASK row.")
            selection = _skills(context["execute_skill_selection"])
            expected = [skill for skill in context["skill_selection"]["skills"] if skill["id"] == task["skill_id"]]
            if (selection["skills"] != expected or len(expected) != (0 if task["skill_id"] is None else 1)
                or context["hierarchy_selection_sha256"] != context["hierarchy_selection"]["selection_sha256"]):
                _fail("Execute must retain the target's exact skill and hierarchy identity.")
            if any(skill["mode_support"]["execute"] == "unsupported" or skill["dependency_status"] != "available" for skill in expected):
                _fail("Execute requires available, supported skills.")
    elif role == "task-skill":
        strict_keys(context, location="task_skill_context", required={"task_boundary", "skill_snapshot", "source_plan",
            "work_instruction_selection", "repository_evidence", "saved_discussion"})
        boundary = _object(context["task_boundary"], "task_boundary")
        if not isinstance(boundary.get("id"), str) or not re.fullmatch(r"TASK-[0-9]{3}", boundary["id"]):
            _fail("Task refinement requires one identified TASK boundary.")
        for field in ("title", "goal"):
            nonempty_string(boundary.get(field), location="task_boundary." + field)
        skill = strict_keys(context["skill_snapshot"], location="skill_snapshot", required=set(SKILL_FIELDS))
        _skills({"schema": "work-skill-selection/v1", "decision": "external_skills", "skills": [skill],
                 "selection_sha256": selection_sha256("external_skills", [skill])})
        plan = _object(context["source_plan"], "source_plan")
        selection = _skills(plan.get("skill_selection"))
        if boundary.get("skill_id") != skill["id"] or skill not in selection["skills"] or skill["mode_support"]["task"] == "unsupported" or skill["dependency_status"] != "available":
            _fail("Task refinement requires exactly its Plan-confirmed executable skill.")
        stored_selection(context["work_instruction_selection"])
        _texts(context["repository_evidence"], "repository_evidence")
        _texts(context["saved_discussion"], "saved_discussion")
    else:
        common = {"requirement_id", "continuation_point"}
        fields = {"content", "expected_revision"} if role == "progress-saver" else {
            "artifacts", "confirmed_request", "decisions", "affected_task_ids", "hierarchy_selection", "skill_selection", "repository_evidence"}
        strict_keys(context, location="maintenance_context", required=common | fields,
                    optional={"save_approval"} if role == "progress-saver" else set())
        requirement = validate_requirement_id(nonempty_string(context["requirement_id"], location="requirement_id"))
        nonempty_string(context["continuation_point"], location="continuation_point")
        if role == "progress-saver":
            revision = context["expected_revision"]
            if type(revision) is not int or revision < 0:
                _fail("Expected saved revision must be a nonnegative integer.")
            content = strict_keys(context["content"], location="content", required=set(FIELDS) - {
                "schema", "requirement_id", "mode", "revision", "status"})
            validate_progress_contract({**content, "schema": "work-discussion-progress/v1", "requirement_id": requirement,
                "mode": mode, "revision": revision + 1, "status": "discussion_only"})
            if "save_approval" in context:
                sha256(context["save_approval"], location="save_approval")
        else:
            paths = context["artifacts"]
            validate_artifact_paths(project_root, requirement, paths, actual_plan_path=paths.get("plan") if isinstance(paths, dict) else "")
            _object(context["confirmed_request"], "confirmed_request")
            _hierarchy(context["hierarchy_selection"])
            _skills(context["skill_selection"])
            if not isinstance(context["decisions"], list) or not context["decisions"]:
                _fail("Artifact revision requires retained confirmed decisions.")
            for decision in context["decisions"]:
                if isinstance(decision, dict):
                    _object(decision, "decision")
                else:
                    nonempty_string(decision, location="decision")
            _texts(context["affected_task_ids"], "affected_task_ids", task_ids=True)
            _texts(context["repository_evidence"], "repository_evidence")
    return DelegationValidationContract(
        schema="work-delegation-validation/v1", status="valid", role=role,
        mode=mode, scope="discussion_restoration" if resume else "role_context",
        source_validation="not_checked", sender_authentication="not_checked",
        grants_authorization=False,
    ).to_canonical_dict()
