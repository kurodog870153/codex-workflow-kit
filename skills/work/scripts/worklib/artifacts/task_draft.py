"""Versioned discussion storage. The current index is the only commit point.

History directories are exclusively reserved and never overwritten or removed.
An interrupted save may leave an uncommitted directory; a later write then
stops for recovery, while readers can still use the last committed index.
Live Plan/skill validation and user authorization remain caller responsibilities.
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import re
import unicodedata
from pathlib import Path
from typing import Any

from ..contracts.task_draft import validate_task_draft, validate_task_planning_index
from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path, validate_requirement_id


def _error(code: str, message: str) -> WorkError:
    return WorkError(ExitCode.WORKFLOW_STATE, code, message)


def _render(value: dict[str, Any]) -> bytes:
    return (unicodedata.normalize("NFC", json.dumps(
        value, ensure_ascii=False, sort_keys=True, indent=2, allow_nan=False,
    )) + "\n").encode("utf-8")


def _path(project_root: Path, requirement_id: str, suffix: str) -> Path:
    validate_requirement_id(requirement_id)
    root = project_root.resolve(strict=True)
    relative = f"outputs/work/tasks/{requirement_id}/drafts"
    # Reject links in the storage chain, including dangling links and junctions.
    # This also prevents a valid project-relative link from aliasing another need.
    candidate = root
    for part in (relative + "/" + suffix).split("/"):
        candidate = candidate / part
        if candidate.is_symlink() or (
            candidate.exists() and getattr(candidate.lstat(), "st_file_attributes", 0) & 0x400
        ):
            raise _error("draft_path_link", "Draft storage paths cannot contain links or junctions.")
    _, resolved = resolve_project_relative_path(root, relative + "/" + suffix)
    if not resolved.is_relative_to(root / relative):
        raise _error("draft_path_escape", "The draft path escapes its requirement directory.")
    return resolved


def _read(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        raise WorkError(ExitCode.IO_FAILURE, "draft_read_failed", "The draft artifact could not be read.", {"path": str(path)}) from error


def _decode(raw: bytes) -> dict[str, Any]:
    value = parse_json_contract(raw, source="draft storage")
    if _render(value) != raw:
        raise _error("noncanonical_draft_storage", "The stored JSON is not canonical.")
    return value


def read_task_planning_index(project_root: Path, requirement_id: str) -> dict[str, Any]:
    """Read the committed index and its historical copy, without any drafts."""
    raw = _read(_path(project_root, requirement_id, "index.json"))
    index = _decode(raw)
    validate_task_planning_index(index)
    if index["requirement_id"] != requirement_id:
        raise _error("draft_requirement_mismatch", "The stored requirement does not match its directory.")
    for task in index["tasks"]:
        if task["status"] != "planned" and "draft_ref" not in task:
            raise _error("missing_stored_draft_reference", "A saved discussion requires a version reference.")
    history = _path(project_root, requirement_id, f"history/{index['revision']}/index.json")
    if _read(history) != raw:
        raise _error("draft_index_integrity", "The current index differs from its immutable history.")
    return index


def read_task_draft(project_root: Path, requirement_id: str, task_id: str) -> dict[str, Any]:
    """Read only the selected historical draft; never trust the display copy."""
    if not isinstance(task_id, str) or not re.fullmatch(r"TASK-[0-9]{3}", task_id):
        raise _error("invalid_draft_task_id", "A TASK-NNN identifier is required.")
    index = read_task_planning_index(project_root, requirement_id)
    entry = next((task for task in index["tasks"] if task["id"] == task_id), None)
    if entry is None or "draft_ref" not in entry:
        raise _error("draft_not_saved", "The requested TASK has no committed draft.")
    reference = entry["draft_ref"]
    raw = _read(_path(project_root, requirement_id, f"history/{reference['save_revision']}/{task_id}.json"))
    if hashlib.sha256(raw).hexdigest() != reference["sha256"]:
        raise _error("draft_content_integrity", "The draft does not match its indexed fingerprint.")
    draft = _decode(raw)
    if draft.get("task_id") != task_id:
        raise _error("draft_task_mismatch", "The historical draft identifies a different TASK.")
    validate_task_draft(draft, index=index)
    return draft


def _write(path: Path, raw: bytes) -> None:
    with path.open("xb") as output:
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())
    if path.read_bytes() != raw:
        raise _error("draft_write_integrity", "The saved bytes do not match the proposed content.")


def _prepare_task_planning(
    index: dict[str, Any],
    *,
    expected_revision: int,
    previous: dict[str, Any] | None,
    draft: dict[str, Any] | None = None,
) -> tuple[dict[str, Any], bytes | None, str | None]:
    """Validate and render the same proposal for both save and recovery."""
    proposed = copy.deepcopy(index)
    validate_task_planning_index(proposed)
    if type(expected_revision) is not int or expected_revision < 0:
        raise _error("invalid_expected_revision", "The expected revision must be a nonnegative integer.")
    if proposed["revision"] != expected_revision + 1:
        raise _error("draft_revision_conflict", "The new index must immediately follow the expected revision.")
    if (previous["revision"] if previous else 0) != expected_revision:
        raise _error("draft_revision_conflict", "The current index changed; reload before saving.")
    if previous is not None and proposed.get("retired_task_ids", []) != previous.get("retired_task_ids", []):
        raise _error("draft_scope_changed", "Discussion saving cannot change retired TASK identifiers.")
    if previous is None:
        if draft is not None or any(task["status"] != "planned" or "draft_ref" in task for task in proposed["tasks"]):
            raise _error("invalid_initial_draft_index", "Initialize the confirmed TASK list before saving discussions.")
    elif draft is None:
        raise _error("draft_required", "Updating an existing index requires one TASK discussion.")
    draft_raw = None
    task_id = None
    if draft is not None:
        candidate = copy.deepcopy(draft)
        # The old reference in the caller's index is replaced only by storage.
        task_id = candidate.get("task_id")
        target = next((task for task in proposed["tasks"] if task["id"] == task_id), None)
        if target is None:
            raise _error("draft_task_not_in_index", "The draft TASK must exist in the proposed index.")
        old_entries = {task["id"]: task for task in previous["tasks"]}
        if proposed["source"] != previous["source"] or list(old_entries) != [task["id"] for task in proposed["tasks"]]:
            raise _error("draft_scope_changed", "This save cannot change the source snapshot or TASK list.")
        for task in proposed["tasks"]:
            if task["id"] != task_id and task != old_entries[task["id"]]:
                raise _error("draft_scope_changed", "This save can change only the selected TASK.")
        old = old_entries[task_id]
        old_revision = old.get("draft_ref", {}).get("revision", 0)
        if candidate.get("revision") != old_revision + 1:
            raise _error("draft_revision_conflict", "The draft must immediately follow its previous revision.")
        boundary_fields = ("title", "goal", "scope", "skill_id", "dependencies", "instructions_sha256")
        changed = any(target[field] != old[field] for field in boundary_fields)
        if target["boundary_revision"] != old["boundary_revision"] + int(changed):
            raise _error("draft_boundary_conflict", "Boundary changes require exactly one new boundary revision.")
        target.pop("draft_ref", None)
        validate_task_draft(candidate, index=proposed)
        draft_raw = _render(candidate)
        target["draft_ref"] = {
            "save_revision": proposed["revision"], "revision": candidate["revision"],
            "sha256": hashlib.sha256(draft_raw).hexdigest(),
        }
    validate_task_planning_index(proposed)
    return proposed, draft_raw, task_id


def save_task_planning(
    project_root: Path,
    index: dict[str, Any],
    *,
    expected_revision: int,
    draft: dict[str, Any] | None = None,
) -> dict[str, object]:
    """Initialize an index or save one discussion against its prior revision."""
    validate_task_planning_index(index)
    requirement_id = index["requirement_id"]
    current_path = _path(project_root, requirement_id, "index.json")
    previous = read_task_planning_index(project_root, requirement_id) if current_path.exists() else None
    proposed, draft_raw, task_id = _prepare_task_planning(
        index, expected_revision=expected_revision, previous=previous, draft=draft,
    )
    raw = _render(proposed)
    revision = proposed["revision"]
    history_prefix = f"history/{revision}"
    history = _path(project_root, requirement_id, history_prefix)
    committed = False
    try:
        history.parent.mkdir(parents=True, exist_ok=True)
        # Exclusive reservation serializes cooperating writers at this revision.
        history.mkdir()
        if draft_raw is not None:
            _write(_path(project_root, requirement_id, f"{history_prefix}/{task_id}.json"), draft_raw)
        _write(_path(project_root, requirement_id, f"{history_prefix}/index.json"), raw)
        prepared = _path(project_root, requirement_id, f"{history_prefix}/index-current.tmp")
        _write(prepared, raw)
        if previous is not None:
            if read_task_planning_index(project_root, requirement_id) != previous:
                raise _error("draft_revision_conflict", "The committed index changed during save.")
        elif current_path.exists():
            raise _error("draft_revision_conflict", "Another initial index appeared during save.")
        os.replace(prepared, _path(project_root, requirement_id, "index.json"))
        committed = True
    except (OSError, WorkError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE, "draft_save_interrupted",
            "Save did not commit. Preserve history for recovery; the prior index remains authoritative.",
            {"revision": revision, "committed": committed, "recovery_required": True},
        ) from error
    mirror_status = "not_applicable"
    if draft_raw is not None:
        try:
            prepared = _path(project_root, requirement_id, f"{history_prefix}/{task_id}-latest.tmp")
            _write(prepared, draft_raw)
            # Do not overwrite a display copy from a later committed revision.
            if read_task_planning_index(project_root, requirement_id)["revision"] != revision:
                mirror_status = "superseded"
            else:
                os.replace(prepared, _path(project_root, requirement_id, f"{task_id}.json"))
                mirror_status = "updated"
        except (OSError, WorkError):
            mirror_status = "stale"
    return {
        "schema": "work-task-draft-save/v1", "requirement_id": requirement_id,
        "revision": revision, "status": "saved", "mirror_status": mirror_status,
    }


def recover_task_planning(
    project_root: Path,
    index: dict[str, Any],
    *,
    expected_revision: int,
    draft: dict[str, Any] | None = None,
) -> dict[str, object]:
    """Publish only a fully prepared, byte-identical authorized save.

Recovery never recreates a missing prepared file. Consuming the original
index-current.tmp allows only one recovering process to publish this version.
Incomplete history and display copies are left untouched.
"""
    validate_task_planning_index(index)
    if type(expected_revision) is not int or expected_revision < 0:
        raise _error("invalid_expected_revision", "The expected revision must be a nonnegative integer.")
    requirement_id = index["requirement_id"]
    previous = None
    if expected_revision:
        previous = _decode(_read(_path(project_root, requirement_id, f"history/{expected_revision}/index.json")))
        validate_task_planning_index(previous)
        if previous["requirement_id"] != requirement_id:
            raise _error("draft_requirement_mismatch", "The previous index belongs to another requirement.")
    proposed, draft_raw, task_id = _prepare_task_planning(
        index, expected_revision=expected_revision, previous=previous, draft=draft,
    )
    revision = proposed["revision"]
    prefix = f"history/{revision}"
    raw = _render(proposed)
    expected_files = {"index.json": raw, "index-current.tmp": raw}
    if draft_raw is not None:
        expected_files[f"{task_id}.json"] = draft_raw
        expected_files[f"{task_id}-latest.tmp"] = draft_raw
    history = _path(project_root, requirement_id, prefix)
    if not history.is_dir():
        raise _error("draft_recovery_incomplete", "The prepared history directory is missing.")
    observed = {path.name for path in history.iterdir()}
    if observed - expected_files.keys():
        raise _error("draft_recovery_conflict", "The history directory contains unknown files.")
    required = {"index.json"} | ({f"{task_id}.json"} if draft_raw is not None else set())
    if not required <= observed:
        raise _error("draft_recovery_incomplete", "The historical index or draft is incomplete.")
    for name in observed:
        if _read(_path(project_root, requirement_id, f"{prefix}/{name}")) != expected_files[name]:
            raise _error("draft_recovery_conflict", "Preserved bytes differ from the original save request.")
    current_path = _path(project_root, requirement_id, "index.json")
    current = read_task_planning_index(project_root, requirement_id) if current_path.exists() else None
    if current == proposed:
        status = "already_completed"
    else:
        if current != previous:
            raise _error("draft_revision_conflict", "Recovery cannot replace a changed or newer index.")
        if "index-current.tmp" not in observed:
            raise _error("draft_recovery_incomplete", "The complete prepared index is required for recovery.")
        # Recheck immediately before consuming the original single-use source.
        if {path.name for path in history.iterdir()} != observed:
            raise _error("draft_recovery_conflict", "The history file set changed during recovery.")
        for name in observed:
            if _read(_path(project_root, requirement_id, f"{prefix}/{name}")) != expected_files[name]:
                raise _error("draft_recovery_conflict", "Prepared content changed during recovery.")
        latest = read_task_planning_index(project_root, requirement_id) if current_path.exists() else None
        if latest != previous:
            raise _error("draft_revision_conflict", "The current index changed during recovery.")
        try:
            os.replace(
                _path(project_root, requirement_id, f"{prefix}/index-current.tmp"),
                _path(project_root, requirement_id, "index.json"),
            )
        except OSError as error:
            raise WorkError(
                ExitCode.IO_FAILURE, "draft_recovery_interrupted",
                "Recovery could not publish the index; preserve the current state for inspection.",
                {"revision": revision, "recovery_required": True},
            ) from error
        status = "recovered"
    return {
        "schema": "work-task-draft-recovery/v1", "requirement_id": requirement_id,
        "revision": revision, "status": status,
        "display_copy": "not_updated" if draft_raw is not None else "not_applicable",
    }
