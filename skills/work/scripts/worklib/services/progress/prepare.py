from __future__ import annotations

import hashlib
from collections.abc import Callable
from pathlib import Path
from typing import Any

from ...technical.infrastructure.json_contract import render_json_contract
from ...models.common.identifiers import IdentifierPolicy
from ...technical.infrastructure.specification_storage import storage_path
from ...models.common.errors import ExitCode, WorkError
from ...models.progress import FIELDS


def preview_progress(project_root: Path, value: object, *, expected_revision: int, validate_progress: Callable[[object], dict[str, Any]], read_progress: Callable[[Path, str, str], dict[str, Any]]) -> dict[str, Any]:
    progress = validate_progress(value)
    if type(expected_revision) is not int or expected_revision < 0:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "The expected revision must be a nonnegative integer.")
    IdentifierPolicy.requirement_id(progress["requirement_id"])
    directory = f"outputs/work/progress/{progress['requirement_id']}/{progress['mode']}"
    relative = directory + "/progress.json"
    previous = read_progress(project_root, progress["requirement_id"], progress["mode"]) if storage_path(project_root, relative).exists() else None
    current_revision = previous["progress"]["revision"] if previous else 0
    if current_revision != expected_revision or progress["revision"] != expected_revision + 1:
        raise WorkError(ExitCode.WORKFLOW_STATE, "progress_revision_conflict", "Read and review current progress before saving the next revision.")
    history = f"{directory}/history/{progress['revision']}"
    if storage_path(project_root, history).exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "progress_save_pending", "An existing uncommitted revision requires review; do not retry or overwrite it.", {"path": history, "committed_revision": current_revision})
    binding = {"schema": "work-progress-save-request/v1", "path": relative, "expected_revision": expected_revision, "previous_sha256": previous["sha256"] if previous else None, "progress": progress}
    return {"schema": "work-progress-preview/v1", "status": "valid", "path": relative, "expected_revision": expected_revision, "approved_sha256": hashlib.sha256(render_json_contract(binding)).hexdigest(), "progress": progress, "source_validation": "not_checked", "evidence_trust": "historical_context_only", "formal_readiness": "not_established"}


def prepare_progress(project_root: Path, value: object, *, requirement_id: str, mode: str, expected_revision: int, preview: Callable[..., dict[str, Any]], validate_keys: Callable[..., dict[str, Any]]) -> dict[str, Any]:
    if type(expected_revision) is not int or expected_revision < 0:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "The expected revision must be a nonnegative integer.")
    IdentifierPolicy.requirement_id(requirement_id)
    if mode not in ("plan", "task"):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_mode", "Only Plan and Task discussions can be saved.")
    content = validate_keys(value, location="progress_prepare", required=set(FIELDS) - {"schema", "requirement_id", "mode", "revision", "status"})
    candidate = {"schema": "work-discussion-progress/v1", "requirement_id": requirement_id, "mode": mode, "revision": expected_revision + 1, "status": "discussion_only", **content}
    return {**preview(project_root, candidate, expected_revision=expected_revision), "schema": "work-progress-prepare/v1"}
