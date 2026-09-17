"""Derive planning metadata from confirmed boundaries and current sources."""

from __future__ import annotations

import copy
from pathlib import Path

from .task_draft import _path, read_task_planning_index, save_task_planning
from .task_draft_list import _prepare_list
from ..services.plan_validation import validate_plan_contract
from ..contracts.task_draft import SOURCE_FIELDS, validate_task_planning_index
from ..contracts.task_draft_models import TaskDraftPrepareContract
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..foundation.runtime import installed_work_root
from ..services.hierarchy_selection import validate_task_hierarchy_paths
from ..services.instruction_draft_selection import resolve_draft_instruction_selection, validate_draft_instruction_selection
from ..services.instruction_selection import build_instruction_selection
from ..services.skill_catalog import SkillRoot


BOUNDARY_FIELDS = {"id", "title", "goal", "scope", "skill_id", "dependencies"}


def _fail(code, message):
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message)


def _boundaries(choices, plan, previous):
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
        work_root = installed_work_root()
        validate_task_hierarchy_paths(selected["selected_paths"], confirmed_selection=plan["hierarchy_selection"],
                                      skill_root=work_root, location="boundary.instruction_selection")
        instruction = build_instruction_selection(skill_root=work_root, mode="task",
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
    expected_revision: int = 0,
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
    normalized, resolved = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    raw = read_raw(resolved)
    options = dict(source=str(resolved), actual_plan_path=normalized, project_root=project_root,
                   user_config_root=user_config_root, skill_roots=skill_roots)
    validation = validate_plan_contract(raw, **options)
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
        edits = _boundaries(payload["upsert"], plan, previous["tasks"])
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
    entries = _boundaries(choices, plan, previous["tasks"] if previous else [])
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
    _, current_path = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    if current_path != resolved or read_raw(current_path) != raw or validate_plan_contract(raw, **options) != validation:
        _fail("draft_source_drift", "The Plan sources changed during preparation.")
    if _boundaries(choices, plan, previous["tasks"] if previous else []) != entries:
        _fail("draft_instruction_drift", "TASK instruction sources changed during preparation.")
    if previous and read_task_planning_index(project_root, requirement_id) != previous:
        _fail("draft_revision_conflict", "The planning index changed during preparation.")
    return TaskDraftPrepareContract.model_validate({"schema": "work-task-draft-prepare/v1", "status": "prepared", "request": prepared_request,
            "index": preview, "affected_task_ids": affected,
            "drafts": {task_id: parse_json_contract(content, source=task_id) for task_id, content in drafts.items()}}).to_canonical_dict()


def initialize_task_planning_request(project_root: Path, requirement_id: str, request: object, *,
                                     plan_path: str, user_config_root: str, skill_roots=None,
                                     prepare_only: bool = False) -> dict[str, object]:
    prepared = prepare_task_planning_request(project_root, requirement_id, request,
        plan_path=plan_path, user_config_root=user_config_root, skill_roots=skill_roots)
    if prepare_only:
        return prepared
    try:
        result = save_task_planning(project_root, prepared["request"], expected_revision=0)
    except WorkError as error:
        # Preserve the exact complete object needed by existing draft-recover.
        error.details["prepared_index"] = prepared["request"]
        raise
    return {**result, "prepared_index": prepared["request"]}
