"""Derive planning metadata from confirmed boundaries and current sources."""

from __future__ import annotations

import copy
from pathlib import Path

from ...services.task.draft.storage import _path, read_task_planning_index, save_task_planning
from .draft_list import _prepare_list
from ...services.task.draft.validation import SOURCE_FIELDS, validate_task_planning_index
from ...models.task_draft import TaskDraftPrepareContract, TaskSemanticRequestContract
from ...models.common.validation import ContractValuePolicy
from ...models.common.errors import ExitCode, WorkError
from ...services.plan.document import parse as parse_json_contract
from ...services.task.storage import read_project_task_source
from ...services.instruction.root import instruction_root
from .hierarchy import validate_task_hierarchy_paths
from ...services.task.draft.validation import resolve_draft_instruction_selection, validate_draft_instruction_selection
from ...services.skill_catalog import SkillRoot


nonempty_string = ContractValuePolicy.nonempty_string
strict_keys = ContractValuePolicy.strict_keys


BOUNDARY_FIELDS = {"id", "title", "goal", "scope", "skill_id", "dependencies"}


def _fail(code, message):
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message)


def _boundaries(choices, plan, previous, operations):
    if not isinstance(choices, list):
        _fail("invalid_draft_array", "Supply an array of confirmed boundaries.")
    old = {entry["id"]: entry for entry in previous}
    skills = {skill["id"]: skill for skill in plan["skill_selection"]["skills"]}
    result, seen = [], set()
    for value in choices:
        choice = strict_keys(value, location="boundary", required=BOUNDARY_FIELDS, optional={"instruction_selection"})
        task_id = nonempty_string(choice["id"], location="id")
        if task_id in seen:
            _fail("duplicate_draft_task_id", "Supply each boundary once.")
        seen.add(task_id)
        original = old.get(task_id)
        explicit = validate_draft_instruction_selection(choice["instruction_selection"]) if "instruction_selection" in choice else None
        selected = resolve_draft_instruction_selection(original or {},
            selected_paths=explicit["selected_paths"] if explicit else None,
            reference_names=explicit["references"] if explicit else None)
        skill_id = choice["skill_id"]
        if skill_id is not None:
            nonempty_string(skill_id, location="skill_id")
            if skill_id not in skills or skills[skill_id]["mode_support"]["task"] == "unsupported":
                _fail("draft_skill_not_available", "Use a Plan-confirmed Task-capable skill.")
        work_root = instruction_root()
        validate_task_hierarchy_paths(selected["selected_paths"], confirmed_selection=plan["hierarchy_selection"],
                                      skill_root=work_root, location="boundary.instruction_selection")
        instruction = operations.build_instruction_selection(skill_root=work_root, mode="task",
            selected_paths=selected["selected_paths"], reference_names=selected["references"])
        if original and instruction["instructions_sha256"] != original["instructions_sha256"]:
            _fail("draft_instruction_drift", "Review instruction drift through source-update first.")
        entry = copy.deepcopy(original) if original else {"status": "planned", "boundary_revision": 1}
        entry.update({field: copy.deepcopy(choice[field]) for field in BOUNDARY_FIELDS})
        entry["instructions_sha256"] = instruction["instructions_sha256"]
        # Legacy selections are checked but not silently added by list updates.
        if not original or "instruction_selection" in original:
            entry["instruction_selection"] = selected
        result.append(entry)
    return result


def prepare_task_planning_request(
    project_root: Path, requirement_id: str, request: object, *, plan_path: str,
    user_config_root: str, skill_roots: list[SkillRoot] | None = None,
    expected_revision: int = 0, operations=None,
) -> dict[str, object]:
    """Prepare initialization or list edits without creating storage."""
    if type(expected_revision) is not int or expected_revision < 0:
        _fail("invalid_expected_revision", "Supply a nonnegative expected revision.")
    initial = expected_revision == 0
    fields = {"tasks", "current_task_id"} if initial else {"upsert", "remove_task_ids", "current_task_id", "reason"}
    payload = strict_keys(request, location="draft_prepare", required=fields)
    previous = None if initial else read_task_planning_index(project_root, requirement_id)
    if previous and previous["revision"] != expected_revision:
        _fail("draft_revision_conflict", "Reload the current index before preparing a list change.")
    index_path = _path(project_root, requirement_id, "index.json")
    if initial and index_path.parent.exists() and any(index_path.parent.iterdir()):
        _fail("draft_initial_storage_exists", "Existing planning storage requires review, not initialization.")
    if not initial and _path(project_root, requirement_id, f"history/{expected_revision + 1}").exists():
        _fail("draft_save_pending", "A reserved revision requires recovery review.")
    normalized, resolved, raw = read_project_task_source(
        project_root, plan_path, field="plan_path"
    )
    options = dict(source=str(resolved), actual_plan_path=normalized, project_root=project_root,
                   user_config_root=user_config_root, skill_roots=skill_roots,
                   _allow_task_index=True)
    validation = operations.validate_plan_contract(raw, **options)
    if validation["requirement_id"] != requirement_id:
        _fail("draft_source_requirement_mismatch", "The Plan belongs to another requirement.")
    source = {key: validation[key] for key in SOURCE_FIELDS}
    if previous and previous["source"] != source:
        _fail("draft_source_drift", "Review changed planning sources before list edits.")
    plan = parse_json_contract(raw, source=str(resolved))
    if initial:
        choices = payload["tasks"]
    else:
        removed = payload["remove_task_ids"]
        if not isinstance(removed, list) or any(not isinstance(item, str) for item in removed) or len(set(removed)) != len(removed):
            _fail("invalid_removed_task_ids", "Supply unique removed TASK IDs.")
        old = {entry["id"]: entry for entry in previous["tasks"]}
        if not set(removed) <= set(old):
            _fail("draft_task_not_in_index", "Only active TASK IDs may be removed.")
        edits = _boundaries(payload["upsert"], plan, previous["tasks"], operations)
        by_id = {entry["id"]: entry for entry in edits}
        if set(removed) & set(by_id):
            _fail("draft_conflicting_edit", "A TASK cannot be removed and updated together.")
        # Check unchanged selections as well; no stale source is promoted by a list edit.
        choices = []
        for entry in previous["tasks"]:
            if entry["id"] in removed:
                continue
            choice = {field: entry[field] for field in BOUNDARY_FIELDS}
            if "instruction_selection" in entry:
                choice["instruction_selection"] = entry["instruction_selection"]
            choices.append(next((item for item in payload["upsert"] if item["id"] == entry["id"]), choice))
        choices.extend(item for item in payload["upsert"] if item["id"] not in old)
    entries = _boundaries(choices, plan, previous["tasks"] if previous else [], operations)
    index = copy.deepcopy(previous) if previous else {"schema": "work-task-planning-index/v1", "requirement_id": requirement_id}
    index.update(revision=expected_revision + 1, source=source,
                 current_task_id=payload["current_task_id"], tasks=entries)
    validate_task_planning_index(index)
    if initial:
        preview, drafts, affected = index, {}, [entry["id"] for entry in entries]
        prepared_request = index
    else:
        preview, drafts, affected = _prepare_list(project_root, previous, index, payload["reason"])
        prepared_request = {"index": index, "reason": payload["reason"]}
    _, current_path, current_raw = read_project_task_source(
        project_root, plan_path, field="plan_path"
    )
    if current_path != resolved or current_raw != raw or operations.validate_plan_contract(raw, **options) != validation:
        _fail("draft_source_drift", "The Plan sources changed during preparation.")
    if _boundaries(choices, plan, previous["tasks"] if previous else [], operations) != entries:
        _fail("draft_instruction_drift", "TASK instruction sources changed during preparation.")
    if previous and read_task_planning_index(project_root, requirement_id) != previous:
        _fail("draft_revision_conflict", "The planning index changed during preparation.")
    return TaskDraftPrepareContract.model_validate({"schema": "work-task-draft-prepare/v1", "status": "prepared", "request": prepared_request,
            "index": preview, "affected_task_ids": affected,
            "drafts": {task_id: parse_json_contract(content, source=task_id) for task_id, content in drafts.items()}}).to_canonical_dict()


def initialize_task_planning_request(project_root: Path, requirement_id: str, request: object, *,
                                     plan_path: str, user_config_root: str, skill_roots=None,
                                     prepare_only: bool = False, operations=None) -> dict[str, object]:
    prepared = prepare_task_planning_request(project_root, requirement_id, request,
        plan_path=plan_path, user_config_root=user_config_root, skill_roots=skill_roots,
        operations=operations)
    if prepare_only:
        return prepared
    try:
        result = save_task_planning(project_root, prepared["request"], expected_revision=0)
    except WorkError as error:
        # Preserve the exact complete object needed by existing draft-recover.
        error.details["prepared_index"] = prepared["request"]
        raise
    return {**result, "prepared_index": prepared["request"]}


def prepare_semantic_task_request(project_root: Path, requirement_id: str, raw: bytes, *, source: str,
                                  plan_path: str, user_config_root: str,
                                  expected_revision: int = 0,
                                  skill_roots: list[SkillRoot] | None = None,
                                  operations=None) -> dict[str, object]:
    semantic = TaskSemanticRequestContract.parse_json_bytes(raw, source=source).to_canonical_dict()
    if type(expected_revision) is not int or expected_revision < 0:
        _fail("invalid_expected_revision", "Supply a nonnegative expected revision.")
    previous = None if expected_revision == 0 else read_task_planning_index(project_root, requirement_id)
    if previous and previous["revision"] != expected_revision:
        _fail("draft_revision_conflict", "Reload the current index before preparing a list change.")
    if not previous and (semantic["remove_task_ids"] or semantic["reason"] is not None):
        _fail("invalid_semantic_initial_request", "Initial TASK planning cannot remove tasks or supply a list-change reason.")
    if previous and (not isinstance(semantic["reason"], str) or not semantic["reason"].strip()):
        _fail("invalid_semantic_reason", "List changes require a non-empty reason.")
    old = {entry["id"]: entry for entry in previous["tasks"]} if previous else {}
    removed = semantic["remove_task_ids"]
    if len(removed) != len(set(removed)) or not set(removed) <= set(old):
        _fail("invalid_removed_task_ids", "Remove each active TASK at most once.")
    highest = max((int(task_id[5:]) for task_id in set(old) | set(previous.get("retired_task_ids", []))), default=0) if previous else 0
    ids, seen_existing = [], set()
    for item in semantic["upsert"]:
        existing = item.get("existing_task_id")
        if existing is not None:
            if existing not in old or existing in removed or existing in seen_existing:
                _fail("invalid_semantic_existing_task", "An upsert may identify one active, unremoved TASK once.")
            seen_existing.add(existing)
            ids.append(existing)
        else:
            highest += 1
            ids.append(f"TASK-{highest:03d}")
    def resolve(reference: dict[str, object]) -> str:
        existing, position = reference.get("existing_task_id"), reference.get("upsert_position")
        if (existing is None) == (position is None):
            _fail("invalid_semantic_task_reference", "A TASK reference needs exactly one existing ID or upsert position.")
        if existing is not None:
            if existing not in old or existing in removed:
                _fail("invalid_semantic_task_reference", "The referenced existing TASK is unavailable.")
            return existing
        if type(position) is not int or position < 1 or position > len(ids):
            _fail("invalid_semantic_task_reference", "The upsert position is out of range.")
        return ids[position - 1]
    upsert = []
    for task_id, item in zip(ids, semantic["upsert"]):
        dependencies = [resolve(reference) for reference in item["dependencies"]]
        if task_id in dependencies:
            _fail("invalid_semantic_task_dependency", "A TASK cannot depend on itself.")
        boundary = {"id": task_id, "title": item["title"], "goal": item["goal"],
                    "scope": item["scope"], "skill_id": item["skill_id"],
                    "dependencies": sorted(set(dependencies))}
        if item.get("instruction_selection") is not None:
            boundary["instruction_selection"] = item["instruction_selection"]
        elif task_id not in old:
            _fail("draft_selection_required", "New TASKs require a confirmed instruction selection.")
        upsert.append(boundary)
    current = resolve(semantic["current_task"]) if semantic["current_task"] is not None else None
    if current in removed:
        _fail("invalid_semantic_current_task", "The current TASK cannot be removed.")
    payload = ({"upsert": upsert, "remove_task_ids": removed, "current_task_id": current,
                "reason": semantic["reason"]} if previous else
               {"tasks": upsert, "current_task_id": current})
    return prepare_task_planning_request(project_root, requirement_id, payload,
        expected_revision=expected_revision, plan_path=plan_path, user_config_root=user_config_root,
        skill_roots=skill_roots, operations=operations)
