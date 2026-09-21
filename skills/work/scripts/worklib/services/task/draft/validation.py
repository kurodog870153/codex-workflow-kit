"""Pure validation for planning indexes and individual, non-executable drafts.

These contracts record discussion progress, not approval or formal readiness.
Source fingerprints are compared with the index; checking live sources and
reading or writing artifacts belong to the future persistence layer.
"""

from __future__ import annotations

import re
from typing import Any

from pydantic import ValidationError

from ....protocol import (
    INVALID_SHA256_ERROR_CODE,
    PLANNING_STATUSES,
    TASK_ID_PATTERN as TASK_ID_PATTERN_TEXT,
)
from ....models.common.errors import ExitCode, WorkError
from ....models.common.identifiers import IdentifierPolicy
from ....models.common.validation import ContractValuePolicy
from ....models.task_draft import (
    TaskDraftContract, TaskDraftValidationContract,
    TaskPlanningIndexContract, TaskPlanningIndexValidationContract,
)


def validate_draft_instruction_selection(value: object) -> dict[str, list[str]]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "instruction_selection must be an object.")
    required = {"selected_paths", "references"}
    if set(value) != required:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": "instruction_selection", "missing": sorted(required - set(value)), "unknown": sorted(set(value) - required)})
    for values in value.values():
        if not isinstance(values, list) or any(not isinstance(item, str) or not item.strip() for item in values) or len(values) != len(set(values)):
            raise WorkError(ExitCode.CONTRACT, "invalid_source_selection", "Instruction selections must be unique string arrays.")
    return {field: list(values) for field, values in value.items()}


def resolve_draft_instruction_selection(entry: dict[str, object], *, selected_paths: list[str] | None = None, reference_names: list[str] | None = None) -> dict[str, list[str]]:
    stored = validate_draft_instruction_selection(entry["instruction_selection"]) if "instruction_selection" in entry else None
    if selected_paths is None:
        if reference_names is not None:
            raise WorkError(ExitCode.CONTRACT, "draft_selection_incomplete", "Explicit references require an explicit instruction path selection.")
        if stored is None:
            raise WorkError(ExitCode.WORKFLOW_STATE, "draft_selection_required", "Confirm instruction paths and references for this legacy TASK before continuing.")
        return stored
    explicit = validate_draft_instruction_selection({"selected_paths": selected_paths, "references": [] if reference_names is None else reference_names})
    if stored is not None and explicit != stored:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_selection_mismatch", "Changing a saved instruction selection requires the source-update workflow.")
    return explicit


def resolve_task_dependencies(
    task_ids: list[str], dependencies: dict[str, list[str]]
) -> tuple[list[str], dict[str, set[str]]]:
    visiting: set[str] = set()
    visited: set[str] = set()
    ancestors: dict[str, set[str]] = {}

    def visit(task_id: str) -> set[str]:
        if task_id in visiting:
            raise WorkError(
                ExitCode.CONTRACT,
                "cyclic_task_dependency",
                "TASK dependencies must not contain a cycle.",
                {"task_id": task_id},
            )
        if task_id in visited:
            return ancestors[task_id]
        visiting.add(task_id)
        result: set[str] = set()
        for dependency in dependencies[task_id]:
            result.add(dependency)
            result.update(visit(dependency))
        visiting.remove(task_id)
        visited.add(task_id)
        ancestors[task_id] = result
        return result

    for task_id in task_ids:
        visit(task_id)
    for task_id, direct in dependencies.items():
        for dependency in direct:
            if any(
                dependency in ancestors[other]
                for other in direct
                if other != dependency
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "indirect_task_dependency",
                    "Only direct TASK dependencies may be listed.",
                    {"task_id": task_id, "dependency": dependency},
                )
    order: list[str] = []
    pending = set(task_ids)
    while pending:
        ready = [
            task_id
            for task_id in task_ids
            if task_id in pending and all(dep in order for dep in dependencies[task_id])
        ]
        if not ready:
            raise WorkError(
                ExitCode.CONTRACT,
                "cyclic_task_dependency",
                "TASK dependency cycle.",
            )
        order.extend(ready)
        pending.difference_update(ready)
    return order, ancestors


nonempty_string = ContractValuePolicy.nonempty_string
sha256 = ContractValuePolicy.sha256
strict_keys = ContractValuePolicy.strict_keys


INDEX_SCHEMA = TaskPlanningIndexContract.contract_id
DRAFT_SCHEMA = TaskDraftContract.contract_id
TASK_ID_PATTERN = re.compile(TASK_ID_PATTERN_TEXT)
SOURCE_FIELDS = {"plan_sha256", "hierarchy_selection_sha256", "skill_selection_sha256"}


def _reject(code: str, message: str, location: str) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, {"location": location})


def _model_error(error: ValidationError, contract: type[TaskDraftContract] | type[TaskPlanningIndexContract]) -> WorkError:
    issue = error.errors(include_url=False, include_context=False, include_input=False)[0]
    if issue["type"] in {"missing", "extra_forbidden"}:
        return contract._work_error(error)
    parts = tuple(issue["loc"])
    location = "".join(
        f"[{part}]" if isinstance(part, int) else f".{part}" if position else str(part)
        for position, part in enumerate(parts)
    ) or "contract"
    field = next((part for part in reversed(parts) if isinstance(part, str)), "")
    if field == "schema":
        return WorkError(ExitCode.CONTRACT, "invalid_draft_schema", "A planning contract schema is required.", {"location": location})
    if field in {"revision", "boundary_revision", "save_revision"}:
        return WorkError(ExitCode.CONTRACT, "invalid_draft_revision", "A positive integer revision is required.", {"location": location})
    if field in {"task_id", "id"}:
        return WorkError(ExitCode.CONTRACT, "invalid_draft_task_id", "A TASK-NNN identifier is required.", {"location": location})
    if field.endswith("sha256"):
        return WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A lowercase SHA-256 fingerprint is required.", {"location": location})
    if field == "status":
        return WorkError(ExitCode.CONTRACT, "invalid_draft_status", "A planning status is required.", {"location": location})
    if field in {"notes", "tentative", "open_questions", "confirmed_decisions", "scope", "dependencies", "retired_task_ids"}:
        return WorkError(ExitCode.CONTRACT, "invalid_draft_array", "A text array with the required cardinality is required.", {"location": location})
    return contract._work_error(error)


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
    IdentifierPolicy.requirement_id(nonempty_string(value["requirement_id"], location="requirement_id"))
    _source(value["source"], "source")


def validate_task_planning_index(value: object) -> dict[str, object]:
    """Validate only the index, without loading any per-TASK draft or skill."""
    try:
        index = TaskPlanningIndexContract.model_validate(value).to_canonical_dict()
    except ValidationError as error:
        raise _model_error(error, TaskPlanningIndexContract) from error
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
    return TaskPlanningIndexValidationContract.model_validate({
        "schema": "work-task-planning-index-validation/v1",
        "requirement_id": index["requirement_id"],
        "revision": index["revision"],
        "task_count": len(task_ids),
        "task_order": order,
        "status": "valid",
    }).to_canonical_dict()


def validate_task_draft(value: object, *, index: object) -> dict[str, object]:
    """Validate one saved discussion against its index, never formalize it.

Notes retain implementation details in progress. Confirmed decisions retain
their rationale separately from tentative ideas and unanswered questions.
Revisions describe data versions; neither revisions nor status grant authority.
"""
    validate_task_planning_index(index)
    assert isinstance(index, dict)
    try:
        draft = TaskDraftContract.model_validate(value).to_canonical_dict()
    except ValidationError as error:
        raise _model_error(error, TaskDraftContract) from error
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
    return TaskDraftValidationContract.model_validate({
        "schema": "work-task-draft-validation/v1",
        "requirement_id": draft["requirement_id"],
        "task_id": task_id,
        "revision": draft["revision"],
        "planning_status": draft["status"],
        "status": "valid",
    }).to_canonical_dict()



__all__ = [
    "DRAFT_SCHEMA", "INDEX_SCHEMA", "SOURCE_FIELDS",
    "resolve_draft_instruction_selection", "resolve_task_dependencies",
    "validate_draft_instruction_selection", "validate_task_draft",
    "validate_task_planning_index",
]

