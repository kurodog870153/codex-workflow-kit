"""Revalidate and version planning source snapshots after explicit review."""

from __future__ import annotations

import copy
import hashlib
import os
from pathlib import Path
from typing import Any

from .task_draft import _decode, _error, _path, _read, _render, _write, read_task_draft_from_index, read_task_planning_index, read_task_planning_revision
from ..contracts.plan import validate_plan_contract
from ..contracts.task_draft import validate_task_draft, validate_task_planning_index
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..foundation.runtime import installed_work_root
from ..hierarchy.selection import validate_task_hierarchy_paths
from ..instructions.selection import build_instruction_selection
from ..instructions.draft_selection import validate_draft_instruction_selection
from ..skills.catalog import SkillRoot


def update_task_draft_sources(
    project_root: Path, requirement_id: str, request: object, *, expected_revision: int,
    plan_path: str, user_config_root: str, skill_roots: list[SkillRoot] | None = None,
    recover: bool = False,
) -> dict[str, object]:
    """Refresh validated fingerprints without changing TASK identities or scope.

The request contains a reason and explicit instruction paths/references for
every active TASK. It cannot supply fingerprints or upgrade a stale Plan.
"""
    payload = strict_keys(request, location="source_update", required={"reason", "selections"})
    nonempty_string(payload["reason"], location="reason")
    if type(expected_revision) is not int or expected_revision < 1:
        raise _error("invalid_expected_revision", "An existing planning revision is required.")
    current = read_task_planning_index(project_root, requirement_id)
    previous = current
    if recover:
        previous = read_task_planning_revision(project_root, requirement_id, expected_revision)
    if previous["revision"] != expected_revision or previous["requirement_id"] != requirement_id:
        raise _error("draft_revision_conflict", "Reload the planning index before updating sources.")
    selections = strict_keys(payload["selections"], location="selections", required={task["id"] for task in previous["tasks"]})
    normalized, resolved = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    raw_plan = read_raw(resolved)
    validation = validate_plan_contract(raw_plan, source=str(resolved), actual_plan_path=normalized,
                                        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    if validation["requirement_id"] != requirement_id:
        raise _error("draft_source_requirement_mismatch", "The validated Plan belongs to another requirement.")
    plan = parse_json_contract(raw_plan, source=str(resolved))
    new_source = {field: validation[field] for field in previous["source"]}
    skills = {skill["id"]: skill for skill in plan["skill_selection"]["skills"]}
    instruction_hashes = {}
    for entry in previous["tasks"]:
        task_id, skill_id = entry["id"], entry["skill_id"]
        if skill_id is not None and (skill_id not in skills or skills[skill_id]["mode_support"]["task"] == "unsupported"):
            raise _error("draft_skill_not_available", "Resolve the TASK skill binding against the confirmed Plan before updating sources.")
        selected = validate_draft_instruction_selection(selections[task_id])
        work_root = installed_work_root()
        validate_task_hierarchy_paths(selected["selected_paths"], confirmed_selection=plan["hierarchy_selection"], skill_root=work_root, location=f"selections.{task_id}")
        instruction = build_instruction_selection(skill_root=work_root, mode="task", selected_paths=selected["selected_paths"], reference_names=selected["references"])
        instruction_hashes[task_id] = instruction["instructions_sha256"]
    global_change = new_source != previous["source"]
    changed = {
        task["id"] for task in previous["tasks"]
        if task["instructions_sha256"] != instruction_hashes[task["id"]]
        or ("instruction_selection" in task and task["instruction_selection"] != selections[task["id"]])
    }
    affected = {task["id"] for task in previous["tasks"]} if global_change else set(changed)
    if not affected:
        raise _error("draft_sources_unchanged", "No source fingerprint changed.")
    while True:
        downstream = {task["id"] for task in previous["tasks"] if set(task["dependencies"]) & affected}
        if downstream <= affected:
            break
        affected |= downstream
    proposed = copy.deepcopy(previous)
    proposed["revision"] += 1
    proposed["source"] = new_source
    drafts = {}
    for entry in proposed["tasks"]:
        task_id = entry["id"]
        entry["instructions_sha256"] = instruction_hashes[task_id]
        entry["instruction_selection"] = validate_draft_instruction_selection(selections[task_id])
        entry["boundary_revision"] += int(task_id in changed)
        if task_id not in affected or "draft_ref" not in entry:
            continue
        draft = read_task_draft_from_index(project_root, previous, task_id)
        draft["source"] = dict(new_source)
        draft["revision"] += 1
        draft["boundary_revision"] = entry["boundary_revision"]
        draft["instructions_sha256"] = entry["instructions_sha256"]
        draft["status"] = entry["status"] = "needs_review"
        draft["notes"].append(f"Sources updated: {payload['reason']}")
        draft["next_discussion_point"] = "Reconfirm affected decisions against the updated sources."
        drafts[task_id] = _render(draft)
        entry["draft_ref"] = {"save_revision": proposed["revision"], "revision": draft["revision"], "sha256": hashlib.sha256(drafts[task_id]).hexdigest()}
    validate_task_planning_index(proposed)
    for raw in drafts.values():
        validate_task_draft(_decode(raw), index=proposed)
    revision = proposed["revision"]
    prefix = f"history/{revision}"
    history = _path(project_root, requirement_id, prefix)
    raw_index = _render(proposed)
    files = {"index.json": raw_index, "index-current.tmp": raw_index,
             "source-update.json": _render({"request": payload, "expected_revision": expected_revision, "plan_path": normalized})}
    files.update({f"{task_id}.json": raw for task_id, raw in drafts.items()})
    if recover:
        if current != previous and current != proposed:
            raise _error("draft_revision_conflict", "Source recovery cannot replace a changed index.")
        if not history.is_dir():
            raise _error("draft_recovery_incomplete", "Prepared source update is missing.")
        observed = {path.name for path in history.iterdir()}
        required = set(files) - ({"index-current.tmp"} if current == proposed else set())
        if observed - set(files) or not required <= observed:
            raise _error("draft_recovery_incomplete", "The complete source-update file set is required.")
        for name in observed:
            if _read(_path(project_root, requirement_id, f"{prefix}/{name}")) != files[name]:
                raise _error("draft_recovery_conflict", "Prepared content differs from the revalidated source update.")
        if current == proposed:
            return {"schema": "work-task-draft-source-update/v1", "status": "already_completed", "revision": revision, "affected_task_ids": sorted(affected)}
    else:
        try:
            history.mkdir()
            for name, content in files.items():
                _write(_path(project_root, requirement_id, f"{prefix}/{name}"), content)
        except (OSError, WorkError) as error:
            raise WorkError(ExitCode.IO_FAILURE, "draft_source_update_interrupted", "Preserve the uncommitted source update for recovery.", {"recovery_required": True}) from error
    if read_task_planning_index(project_root, requirement_id) != previous:
        raise _error("draft_revision_conflict", "The planning index changed before source publication.")
    if {path.name for path in history.iterdir()} != set(files):
        raise _error("draft_recovery_conflict", "The prepared file set changed before publication.")
    for name, content in files.items():
        if _read(_path(project_root, requirement_id, f"{prefix}/{name}")) != content:
            raise _error("draft_recovery_conflict", "The prepared source-update bytes changed.")
    try:
        os.replace(_path(project_root, requirement_id, f"{prefix}/index-current.tmp"), _path(project_root, requirement_id, "index.json"))
    except OSError as error:
        raise WorkError(ExitCode.IO_FAILURE, "draft_source_update_interrupted", "The prepared source update could not be published.", {"recovery_required": True}) from error
    return {"schema": "work-task-draft-source-update/v1", "status": "recovered" if recover else "saved",
            "requirement_id": requirement_id, "revision": revision, "affected_task_ids": sorted(affected), "display_copies": "not_updated"}
