"""Pure validation for planning indexes and individual, non-executable drafts.

These contracts record discussion progress, not approval or formal readiness.
Source fingerprints are compared with the index; checking live sources and
reading or writing artifacts belong to the future persistence layer.
"""

from __future__ import annotations

import re
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.paths import validate_requirement_id
from ..instructions.draft_selection import validate_draft_instruction_selection
from .task_dependencies import resolve_task_dependencies
from .validation import nonempty_string, sha256, strict_keys


INDEX_SCHEMA = "work-task-planning-index/v1"
DRAFT_SCHEMA = "work-task-draft/v1"
TASK_ID_PATTERN = re.compile(r"^TASK-\d{3}$")
PLANNING_STATUSES = ("planned", "in_progress", "refined", "needs_review")
SOURCE_FIELDS = {"plan_sha256", "hierarchy_selection_sha256", "skill_selection_sha256"}


def _reject(code: str, message: str, location: str) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, {"location": location})


def _revision(value: object, location: str) -> None:
    if type(value) is not int or value < 1:
        _reject("invalid_draft_revision", "A positive integer revision is required.", location)


def _task_id(value: object, location: str) -> str:
    text = nonempty_string(value, location=location)
    if not TASK_ID_PATTERN.fullmatch(text):
        _reject("invalid_draft_task_id", "A TASK-NNN identifier is required.", location)
    return text


def _texts(value: object, location: str, *, required: bool = False) -> list[str]:
    if not isinstance(value, list) or (required and not value):
        _reject("invalid_draft_array", "A text array with the required cardinality is required.", location)
    return [nonempty_string(item, location=f"{location}[{i}]") for i, item in enumerate(value)]


def _source(value: object, location: str) -> None:
    source = strict_keys(value, location=location, required=SOURCE_FIELDS)
    for field in SOURCE_FIELDS:
        sha256(source[field], location=f"{location}.{field}")


def _identity(value: dict[str, Any], schema: str) -> None:
    if value["schema"] != schema:
        _reject("invalid_draft_schema", "A planning contract schema is required.", "schema")
    validate_requirement_id(nonempty_string(value["requirement_id"], location="requirement_id"))
    _source(value["source"], "source")


def validate_task_planning_index(value: object) -> dict[str, object]:
    """Validate only the index, without loading any per-TASK draft or skill."""
    index = strict_keys(
        value,
        location="index",
        required={"schema", "requirement_id", "revision", "source", "current_task_id", "tasks"},
        optional={"retired_task_ids"},
    )
    _identity(index, INDEX_SCHEMA)
    _revision(index["revision"], "revision")
    if not isinstance(index["tasks"], list) or not index["tasks"]:
        _reject("invalid_draft_array", "The planning index requires TASK entries.", "tasks")
    task_ids: list[str] = []
    dependencies: dict[str, list[str]] = {}
    for position, raw in enumerate(index["tasks"]):
        location = f"tasks[{position}]"
        task = strict_keys(
            raw,
            location=location,
            required={
                "id", "title", "goal", "scope", "skill_id", "dependencies",
                "status", "boundary_revision", "instructions_sha256",
            },
            optional={"draft_ref", "instruction_selection"},
        )
        task_id = _task_id(task["id"], f"{location}.id")
        if task_id in task_ids:
            _reject("duplicate_draft_task_id", "TASK identifiers must be unique.", location)
        task_ids.append(task_id)
        for field in ("title", "goal"):
            nonempty_string(task[field], location=f"{location}.{field}")
        _texts(task["scope"], f"{location}.scope", required=True)
        if task["skill_id"] is not None:
            nonempty_string(task["skill_id"], location=f"{location}.skill_id")
        if task["status"] not in PLANNING_STATUSES:
            _reject("invalid_draft_status", "A planning status is required.", location)
        _revision(task["boundary_revision"], f"{location}.boundary_revision")
        sha256(task["instructions_sha256"], location=f"{location}.instructions_sha256")
        if "instruction_selection" in task:
            validate_draft_instruction_selection(task["instruction_selection"])
        if "draft_ref" in task:
            reference = strict_keys(
                task["draft_ref"], location=f"{location}.draft_ref",
                required={"save_revision", "revision", "sha256"},
            )
            for field in ("save_revision", "revision"):
                _revision(reference[field], f"{location}.draft_ref.{field}")
            sha256(reference["sha256"], location=f"{location}.draft_ref.sha256")
            if reference["save_revision"] > index["revision"] or task["status"] == "planned":
                _reject("invalid_draft_reference", "A draft reference must identify an existing discussion version.", location)
        direct = _texts(task["dependencies"], f"{location}.dependencies")
        if len(set(direct)) != len(direct):
            _reject("duplicate_draft_dependency", "Dependencies must be unique.", location)
        dependencies[task_id] = direct
    for task_id, direct in dependencies.items():
        if any(dependency not in task_ids or dependency == task_id for dependency in direct):
            _reject("invalid_task_dependency", "A TASK dependency is unknown or refers to itself.", task_id)
    order, _ = resolve_task_dependencies(task_ids, dependencies)
    retired = _texts(index.get("retired_task_ids", []), "retired_task_ids")
    if len(retired) != len(set(retired)) or set(retired) & set(task_ids):
        _reject("invalid_retired_task_ids", "Retired TASK IDs must be unique and absent from the active list.", "retired_task_ids")
    for retired_id in retired:
        _task_id(retired_id, "retired_task_ids")
    if index["current_task_id"] is not None:
        current = _task_id(index["current_task_id"], "current_task_id")
        if current not in task_ids:
            _reject("unknown_current_task", "The resume TASK must exist in the index.", "current_task_id")
    return {
        "schema": "work-task-planning-index-validation/v1",
        "requirement_id": index["requirement_id"],
        "revision": index["revision"],
        "task_count": len(task_ids),
        "task_order": order,
        "status": "valid",
    }


def validate_task_draft(value: object, *, index: object) -> dict[str, object]:
    """Validate one saved discussion against its index, never formalize it.

Notes retain implementation details in progress. Confirmed decisions retain
their rationale separately from tentative ideas and unanswered questions.
Revisions describe data versions; neither revisions nor status grant authority.
"""
    validate_task_planning_index(index)
    assert isinstance(index, dict)
    draft = strict_keys(
        value,
        location="draft",
        required={
            "schema", "requirement_id", "task_id", "revision", "boundary_revision",
            "source", "instructions_sha256", "status", "notes", "confirmed_decisions",
            "tentative", "open_questions", "next_discussion_point",
        },
        optional={"task_candidate"},
    )
    _identity(draft, DRAFT_SCHEMA)
    task_id = _task_id(draft["task_id"], "task_id")
    _revision(draft["revision"], "revision")
    _revision(draft["boundary_revision"], "boundary_revision")
    sha256(draft["instructions_sha256"], location="instructions_sha256")
    if draft["status"] not in PLANNING_STATUSES[1:]:
        _reject("invalid_draft_status", "A saved discussion requires a draft status.", "status")
    for field in ("notes", "tentative", "open_questions"):
        _texts(draft[field], field)
    decisions = draft["confirmed_decisions"]
    if not isinstance(decisions, list):
        _reject("invalid_draft_array", "Confirmed decisions must be an array.", "confirmed_decisions")
    for position, raw in enumerate(decisions):
        location = f"confirmed_decisions[{position}]"
        decision = strict_keys(raw, location=location, required={"statement", "rationale"})
        for field in ("statement", "rationale"):
            nonempty_string(decision[field], location=f"{location}.{field}")
    next_point = draft["next_discussion_point"]
    if next_point is not None:
        nonempty_string(next_point, location="next_discussion_point")
    if draft["status"] == "refined":
        if draft["tentative"] or draft["open_questions"] or next_point is not None:
            _reject("unfinished_refined_draft", "A refined discussion cannot contain pending decisions or questions.", "status")
    elif next_point is None:
        _reject("missing_draft_resume_point", "An unfinished discussion requires a resume point.", "next_discussion_point")
    if draft["requirement_id"] != index["requirement_id"] or draft["source"] != index["source"]:
        _reject("draft_source_mismatch", "The draft must match the index requirement and source fingerprints.", "source")
    entry = next((task for task in index["tasks"] if task["id"] == task_id), None)
    if entry is None:
        _reject("draft_task_not_in_index", "The draft TASK must exist in the index.", "task_id")
    for field in ("boundary_revision", "instructions_sha256", "status"):
        if draft[field] != entry[field]:
            _reject("draft_index_mismatch", "The draft must match its indexed boundary, instructions and status.", field)
    if "draft_ref" in entry and draft["revision"] != entry["draft_ref"]["revision"]:
        _reject("draft_index_mismatch", "The draft revision must match its index reference.", "revision")
    if "task_candidate" in draft:
        candidate = draft["task_candidate"]
        if not isinstance(candidate, dict):
            _reject("invalid_task_candidate", "The structured TASK candidate must be an object.", "task_candidate")
        if draft["status"] == "refined":
            for field in ("id", "title", "goal", "skill_id"):
                if field not in candidate or candidate[field] != entry[field]:
                    _reject("task_candidate_boundary_mismatch", "The candidate differs from its confirmed TASK boundary.", field)
            if candidate.get("dependencies", []) != entry["dependencies"]:
                _reject("task_candidate_boundary_mismatch", "The candidate dependencies differ from the index.", "dependencies")
            selection = candidate.get("instruction_selection")
            if not isinstance(selection, dict) or selection.get("instructions_sha256") != entry["instructions_sha256"]:
                _reject("task_candidate_boundary_mismatch", "The candidate instruction fingerprint differs from the index.", "instruction_selection")
    return {
        "schema": "work-task-draft-validation/v1",
        "requirement_id": draft["requirement_id"],
        "task_id": task_id,
        "revision": draft["revision"],
        "planning_status": draft["status"],
        "status": "valid",
    }
