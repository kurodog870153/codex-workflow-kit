"""Pure validation for planning indexes and individual, non-executable drafts.

These contracts record discussion progress, not approval or formal readiness.
Source fingerprints are compared with the index; checking live sources and
reading or writing artifacts belong to the future persistence layer.
"""

from __future__ import annotations

import copy
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


TASK_CANDIDATE_FIELDS = {"steps", "validations", "files", "commands", "decisions", "inputs", "risks", "operations"}
_CANDIDATE_ITEMS = {
    "inputs": ({"key", "kind", "precondition"}, {"source", "dependency_position", "file_key"}),
    "decisions": ({"key", "statement", "rationale"}, set()),
    "files": ({"key", "action"}, {"path", "source", "destination"}),
    "risks": ({"key", "condition", "impact", "mitigation"}, set()),
    "steps": ({"key", "action", "references"}, set()),
    "commands": ({"key", "mode"}, {"argv", "script", "execution"}),
    "operations": ({"key", "kind", "action", "target", "validation_key"}, {"command_key"}),
    "validations": ({"key", "kind"}, {"command_keys", "pass_condition", "confirmer", "criteria", "acceptance_positions"}),
}
_CANDIDATE_PREFIX = {"inputs": "INPUT", "decisions": "TASK-DECISION", "files": "FILE", "risks": "RISK", "steps": "STEP", "commands": "CMD", "operations": "OP", "validations": "VAL"}


def _candidate_key(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[a-z][a-z0-9_-]*", value):
        raise WorkError(ExitCode.CONTRACT, "invalid_semantic_key", "Use a local lowercase semantic key, not a formal ID.", {"location": location})
    return value


def _positions(value: object, *, location: str, size: int) -> list[int]:
    if not isinstance(value, list) or not value or any(type(position) is not int or position < 1 or position > size for position in value) or len(value) != len(set(value)):
        raise WorkError(ExitCode.CONTRACT, "invalid_semantic_position", "Positions must uniquely identify existing one-based items.", {"location": location})
    return value


def validate_semantic_task_candidate(value: object, *, refined: bool) -> dict[str, Any]:
    candidate = strict_keys(value, location="task_candidate",
        required={"steps", "validations"} if refined else set(),
        optional=TASK_CANDIDATE_FIELDS)
    keys: dict[str, dict[str, str]] = {}
    for group, (required, optional) in _CANDIDATE_ITEMS.items():
        if group not in candidate:
            continue
        rows = candidate[group]
        if not isinstance(rows, list) or (refined and not rows):
            raise WorkError(ExitCode.CONTRACT, "invalid_semantic_items", "A refined candidate group must be a nonempty array.", {"location": group})
        keys[group] = {}
        for position, raw in enumerate(rows, 1):
            row = strict_keys(raw, location=f"task_candidate.{group}[{position}]", required=required, optional=optional)
            key = _candidate_key(row["key"], location=f"{group}[{position}].key")
            if key in keys[group]:
                raise WorkError(ExitCode.CONTRACT, "duplicate_semantic_key", "Local semantic keys must be unique within a group.", {"location": group, "key": key})
            keys[group][key] = f"{_CANDIDATE_PREFIX[group]}-{position:03d}"
            if group == "steps":
                if not isinstance(row["references"], list) or not row["references"]:
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "A step requires semantic references.")
                for reference in row["references"]:
                    strict_keys(reference, location="step.reference", required={"kind", "key"})
            if group == "validations" and "command_keys" in row:
                if not isinstance(row["command_keys"], list) or not row["command_keys"]:
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Automated validations require command keys.")
            if group == "validations" and "acceptance_positions" in row:
                positions = row["acceptance_positions"]
                if not isinstance(positions, list) or not positions or any(type(item) is not int or item < 1 for item in positions) or len(positions) != len(set(positions)):
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_position", "Acceptance positions must be unique positive integers.")
    def resolve(group: str, key: object) -> str:
        local = _candidate_key(key, location=group)
        if local not in keys.get(group, {}):
            raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "A local semantic reference is unknown.", {"group": group, "key": local})
        return keys[group][local]
    for row in candidate.get("steps", []):
        for reference in row["references"]:
            group = reference["kind"]
            if group not in _CANDIDATE_ITEMS or group == "steps":
                raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "A step reference kind is unsupported.")
            resolve(group, reference["key"])
    for row in candidate.get("validations", []):
        for key in row.get("command_keys", []):
            resolve("commands", key)
    for row in candidate.get("operations", []):
        resolve("validations", row["validation_key"])
        if "command_key" in row:
            resolve("commands", row["command_key"])
    return copy.deepcopy(candidate)


def build_semantic_task_candidate(value: object, *, acceptance_ids: list[str], dependency_ids: list[str] | None = None, dependency_files: dict[str, dict[str, str]] | None = None) -> dict[str, Any]:
    candidate = validate_semantic_task_candidate(value, refined=True)
    maps = {group: {row["key"]: f"{_CANDIDATE_PREFIX[group]}-{position:03d}" for position, row in enumerate(candidate.get(group, []), 1)} for group in _CANDIDATE_ITEMS}
    def lookup(group: str, key: str) -> str:
        return maps[group][key]
    result: dict[str, Any] = {}
    for group in _CANDIDATE_ITEMS:
        if group not in candidate:
            continue
        rows = []
        for position, original in enumerate(candidate[group], 1):
            row = {field: copy.deepcopy(item) for field, item in original.items() if field != "key"}
            row["id"] = f"{_CANDIDATE_PREFIX[group]}-{position:03d}"
            if group == "steps":
                row["references"] = [lookup(reference["kind"], reference["key"]) for reference in row["references"]]
            elif group == "validations":
                if "command_keys" in row:
                    row["command_ids"] = [lookup("commands", key) for key in row.pop("command_keys")]
                if "acceptance_positions" in row:
                    row["acceptance_ids"] = [acceptance_ids[item - 1] for item in _positions(row.pop("acceptance_positions"), location="acceptance_positions", size=len(acceptance_ids))]
            elif group == "operations":
                row["validation_id"] = lookup("validations", row.pop("validation_key"))
                if "command_key" in row:
                    row["command_id"] = lookup("commands", row.pop("command_key"))
            elif group == "inputs" and row["kind"] == "task_output":
                dependencies = dependency_ids or []
                files = dependency_files or {}
                if set(row) != {"id", "kind", "precondition", "dependency_position", "file_key"}:
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Task-output input requires a dependency position and file key.")
                dep_position = _positions([row.pop("dependency_position")], location="dependency_position", size=len(dependencies))[0]
                dep_id = dependencies[dep_position - 1]
                file_key = _candidate_key(row.pop("file_key"), location="file_key")
                if file_key not in files.get(dep_id, {}):
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Dependency file key is unknown.")
                row["source"] = f"{dep_id}/{files[dep_id][file_key]}"
            elif group == "inputs" and ("dependency_position" in row or "file_key" in row):
                raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Only task-output inputs use dependency file references.")
            rows.append(row)
        result[group] = rows
    return result


def build_semantic_task_patch(
    current: dict[str, Any], replacements: dict[str, list[dict[str, Any]] | None],
    *, acceptance_ids: list[str], dependency_ids: list[str] | None = None,
    dependency_files: dict[str, dict[str, str]] | None = None,
) -> dict[str, list[dict[str, Any]] | None]:
    """Resolve local keys against retained records and allocate new nested IDs."""
    aliases: dict[str, dict[str, str]] = {}
    for group, prefix in _CANDIDATE_PREFIX.items():
        aliases[group] = {
            f"existing-{position}": row["id"]
            for position, row in enumerate(current.get(group) or [], 1)
        }
        if group not in replacements or replacements[group] is None:
            continue
        rows = replacements[group]
        if not isinstance(rows, list):
            raise WorkError(ExitCode.CONTRACT, "invalid_semantic_items", "Nested semantic edits require arrays.")
        used: set[int] = set()
        next_number = max((int(row["id"].rsplit("-", 1)[1]) for row in current.get(group) or []), default=0)
        aliases[group] = {}
        for position, row in enumerate(rows, 1):
            if not isinstance(row, dict):
                raise WorkError(ExitCode.CONTRACT, "invalid_semantic_items", "Nested semantic rows must be objects.")
            key = _candidate_key(row.get("key"), location=f"{group}[{position}].key")
            if key in aliases[group]:
                raise WorkError(ExitCode.CONTRACT, "duplicate_semantic_key", "Local semantic keys must be unique.")
            old_position = row.get("existing_position")
            if old_position is not None:
                if type(old_position) is not int or old_position < 1 or old_position > len(current.get(group) or []) or old_position in used:
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_position", "An existing position must identify one retained record.")
                used.add(old_position)
                record_id = current[group][old_position - 1]["id"]
                if f"existing-{old_position}" in aliases[group] or key == f"existing-{old_position}":
                    raise WorkError(ExitCode.CONTRACT, "duplicate_semantic_key", "Local semantic keys must be unique.")
                aliases[group][f"existing-{old_position}"] = record_id
            else:
                next_number += 1
                record_id = f"{prefix}-{next_number:03d}"
            aliases[group][key] = record_id
    def resolve(group: str, key: object) -> str:
        name = _candidate_key(key, location=group)
        try:
            return aliases[group][name]
        except KeyError as error:
            raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "A local semantic reference is unknown.", {"group": group, "key": name}) from error
    result: dict[str, list[dict[str, Any]] | None] = {}
    for group, rows in replacements.items():
        if group not in _CANDIDATE_ITEMS:
            raise WorkError(ExitCode.CONTRACT, "invalid_semantic_items", "This nested group is unsupported.")
        if rows is None:
            result[group] = None
            continue
        required, optional = _CANDIDATE_ITEMS[group]
        formal_rows = []
        for position, original in enumerate(rows, 1):
            row = strict_keys(original, location=f"semantic_after.{group}[{position}]",
                              required=required, optional=optional | {"existing_position"})
            formal = {key: copy.deepcopy(value) for key, value in row.items() if key not in {"key", "existing_position"}}
            formal["id"] = resolve(group, row["key"])
            if group == "steps":
                if not isinstance(row["references"], list) or not row["references"]:
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Steps require semantic references.")
                formal["references"] = [resolve(reference["kind"], reference["key"])
                                        for value in row["references"]
                                        for reference in [strict_keys(value, location="step.reference", required={"kind", "key"})]]
            elif group == "validations":
                if "command_keys" in formal:
                    formal["command_ids"] = [resolve("commands", key) for key in formal.pop("command_keys")]
                if "acceptance_positions" in formal:
                    formal["acceptance_ids"] = [acceptance_ids[item - 1] for item in _positions(formal.pop("acceptance_positions"), location="acceptance_positions", size=len(acceptance_ids))]
            elif group == "operations":
                formal["validation_id"] = resolve("validations", formal.pop("validation_key"))
                if "command_key" in formal:
                    formal["command_id"] = resolve("commands", formal.pop("command_key"))
            elif group == "inputs" and formal["kind"] == "task_output":
                dependencies = dependency_ids or []
                files = dependency_files or {}
                dep_position = _positions([formal.pop("dependency_position")], location="dependency_position", size=len(dependencies))[0]
                file_key = _candidate_key(formal.pop("file_key"), location="file_key")
                dep_id = dependencies[dep_position - 1]
                if file_key not in files.get(dep_id, {}):
                    raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Dependency file key is unknown.")
                formal["source"] = f"{dep_id}/{files[dep_id][file_key]}"
            elif group == "inputs" and ("dependency_position" in formal or "file_key" in formal):
                raise WorkError(ExitCode.CONTRACT, "invalid_semantic_reference", "Only task-output inputs use dependency file references.")
            formal_rows.append(formal)
        result[group] = formal_rows
    return result


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
        validate_semantic_task_candidate(draft["task_candidate"], refined=draft["status"] == "refined")
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
    "validate_draft_instruction_selection", "validate_semantic_task_candidate", "build_semantic_task_candidate", "build_semantic_task_patch", "validate_task_draft",
    "validate_task_planning_index",
]
