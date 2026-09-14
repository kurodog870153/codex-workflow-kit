"""Versioned TASK list changes with preserved discussion history."""

from __future__ import annotations

import copy
import hashlib
import os
from pathlib import Path
from typing import Any

from .task_draft import _decode, _error, _path, _read, _render, _write, read_task_planning_index
from ..contracts.task_draft import validate_task_draft, validate_task_planning_index
from ..contracts.validation import nonempty_string
from ..foundation.errors import ExitCode, WorkError


BOUNDARY_FIELDS = ("title", "goal", "scope", "skill_id", "dependencies", "instructions_sha256")


def _prepare_list(
    project_root: Path, previous: dict[str, Any], proposed: dict[str, Any], reason: str,
) -> tuple[dict[str, Any], dict[str, bytes], list[str]]:
    validate_task_planning_index(proposed)
    nonempty_string(reason, location="reason")
    result = copy.deepcopy(proposed)
    if result["requirement_id"] != previous["requirement_id"] or result["source"] != previous["source"]:
        raise _error("draft_scope_changed", "A list update cannot change its requirement or source snapshot.")
    if result["revision"] != previous["revision"] + 1:
        raise _error("draft_revision_conflict", "The list revision must immediately follow the previous index.")
    old = {task["id"]: task for task in previous["tasks"]}
    new = {task["id"]: task for task in result["tasks"]}
    for task_id in set(old) & set(new):
        if new[task_id].get("instruction_selection") != old[task_id].get("instruction_selection"):
            raise _error("draft_selection_mismatch", "List updates must preserve existing instruction selections; use source-update to change them.")
    retired = set(previous.get("retired_task_ids", []))
    added, removed = set(new) - set(old), set(old) - set(new)
    highest = max(int(task_id[5:]) for task_id in set(old) | retired)
    if any(int(task_id[5:]) <= highest for task_id in added):
        raise _error("draft_task_id_reused", "New TASK IDs must exceed all previously allocated IDs.")
    changed = {task_id for task_id in set(new) & set(old) if any(new[task_id][field] != old[task_id][field] for field in BOUNDARY_FIELDS)}
    if not added and not removed and not changed:
        raise _error("draft_list_unchanged", "A list update requires a boundary, addition or removal change.")
    affected = changed | added | removed
    while True:
        downstream = {task_id for task_id, entry in {**old, **new}.items() if set(entry["dependencies"]) & affected}
        # Also account for dependencies removed by this update.
        downstream |= {task_id for task_id, entry in old.items() if set(entry["dependencies"]) & affected}
        if downstream <= affected:
            break
        affected |= downstream
    result["retired_task_ids"] = sorted(retired | removed)
    drafts: dict[str, bytes] = {}
    for task_id, entry in new.items():
        if task_id in added:
            if entry["status"] != "planned" or entry["boundary_revision"] != 1 or "draft_ref" in entry:
                raise _error("invalid_new_draft_task", "New TASKs must be planned at boundary revision 1 without a draft.")
            continue
        original = old[task_id]
        # Status, references and revisions are generated, never caller-authored.
        for field in ("status", "boundary_revision", "draft_ref"):
            if entry.get(field) != original.get(field):
                raise _error("draft_list_metadata_changed", "Preserve original progress metadata in the list proposal.")
        if task_id not in affected:
            continue
        entry["boundary_revision"] += int(task_id in changed)
        if "draft_ref" not in original:
            continue
        reference = original["draft_ref"]
        raw = _read(_path(project_root, result["requirement_id"], f"history/{reference['save_revision']}/{task_id}.json"))
        if hashlib.sha256(raw).hexdigest() != reference["sha256"]:
            raise _error("draft_content_integrity", "The affected historical draft differs from its fingerprint.")
        discussion = _decode(raw)
        if discussion.get("task_id") != task_id:
            raise _error("draft_task_mismatch", "The historical draft belongs to another TASK.")
        validate_task_draft(discussion, index=previous)
        discussion["revision"] += 1
        discussion["boundary_revision"] = entry["boundary_revision"]
        discussion["instructions_sha256"] = entry["instructions_sha256"]
        discussion["status"] = entry["status"] = "needs_review"
        discussion["notes"].append(f"TASK list changed: {reason}")
        discussion["next_discussion_point"] = f"Reconfirm the affected discussion after the TASK list change: {reason}"
        entry.pop("draft_ref")
        drafts[task_id] = _render(discussion)
        entry["draft_ref"] = {"save_revision": result["revision"], "revision": discussion["revision"], "sha256": hashlib.sha256(drafts[task_id]).hexdigest()}
    validate_task_planning_index(result)
    for raw in drafts.values():
        validate_task_draft(_decode(raw), index=result)
    return result, drafts, sorted(affected & set(new))


def update_task_planning_list(
    project_root: Path, index: dict[str, Any], *, expected_revision: int, reason: str, recover: bool = False,
) -> dict[str, object]:
    """Save an authorized list revision or recover its identical prepared bytes."""
    validate_task_planning_index(index)
    if type(expected_revision) is not int or expected_revision < 1:
        raise _error("invalid_expected_revision", "A list update requires an existing index revision.")
    requirement_id = index["requirement_id"]
    current = read_task_planning_index(project_root, requirement_id)
    if recover:
        previous = _decode(_read(_path(project_root, requirement_id, f"history/{expected_revision}/index.json")))
        validate_task_planning_index(previous)
    else:
        previous = current
    if previous["revision"] != expected_revision:
        raise _error("draft_revision_conflict", "Reload the planning index before updating its list.")
    proposed, drafts, affected = _prepare_list(project_root, previous, index, reason)
    raw = _render(proposed)
    prefix = f"history/{proposed['revision']}"
    history = _path(project_root, requirement_id, prefix)
    request = _render({"index": index, "reason": reason, "expected_revision": expected_revision})
    files = {"index.json": raw, "index-current.tmp": raw, "list-update.json": request}
    files.update({f"{task_id}.json": content for task_id, content in drafts.items()})
    if recover:
        if current != previous and current != proposed:
            raise _error("draft_revision_conflict", "Recovery cannot replace a newer or different index.")
        if not history.is_dir():
            raise _error("draft_recovery_incomplete", "The prepared list update is missing.")
        observed = {path.name for path in history.iterdir()}
        required = set(files) - ({"index-current.tmp"} if current == proposed else set())
        if observed - set(files) or not required <= observed:
            raise _error("draft_recovery_incomplete", "Recovery requires the exact complete list-update file set.")
        for name in observed:
            if _read(_path(project_root, requirement_id, f"{prefix}/{name}")) != files[name]:
                raise _error("draft_recovery_conflict", "Preserved content differs from the authorized list update.")
        if current == proposed:
            return {"schema": "work-task-draft-list-update/v1", "status": "already_completed", "revision": proposed["revision"], "affected_task_ids": affected}
    else:
        try:
            history.mkdir()
            for name, content in files.items():
                _write(_path(project_root, requirement_id, f"{prefix}/{name}"), content)
        except (OSError, WorkError) as error:
            raise WorkError(ExitCode.IO_FAILURE, "draft_list_update_interrupted", "The list update was not committed; preserve its history for recovery.", {"recovery_required": True}) from error
    if read_task_planning_index(project_root, requirement_id) != previous:
        raise _error("draft_revision_conflict", "The current index changed before list publication.")
    if {path.name for path in history.iterdir()} != set(files):
        raise _error("draft_recovery_conflict", "The prepared file set changed before list publication.")
    for name, content in files.items():
        if _read(_path(project_root, requirement_id, f"{prefix}/{name}")) != content:
            raise _error("draft_recovery_conflict", "Prepared content changed before list publication.")
    try:
        os.replace(_path(project_root, requirement_id, f"{prefix}/index-current.tmp"), _path(project_root, requirement_id, "index.json"))
    except OSError as error:
        raise WorkError(ExitCode.IO_FAILURE, "draft_list_update_interrupted", "The prepared list update could not be published.", {"recovery_required": True}) from error
    return {
        "schema": "work-task-draft-list-update/v1", "status": "recovered" if recover else "saved",
        "requirement_id": requirement_id, "revision": proposed["revision"], "affected_task_ids": affected,
        "display_copies": "not_updated",
    }
