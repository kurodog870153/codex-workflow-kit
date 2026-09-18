"""Publish one complete discussion snapshot; retain each previous revision."""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from ..contracts.progress import FIELDS, validate_progress_contract
from ..contracts.validation import sha256, strict_keys
from ..models.common.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import validate_requirement_id
from ..foundation.spec_update import storage_path
from ..infrastructure.progress_storage import publish_progress_revision, read_progress_bytes


def _directory(requirement_id: str, mode: str) -> str:
    validate_requirement_id(requirement_id)
    if mode not in ("plan", "task"):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_mode", "Only Plan and Task discussions can be saved.")
    return f"outputs/work/progress/{requirement_id}/{mode}"


def read_progress(project_root: Path, requirement_id: str, mode: str) -> dict[str, Any]:
    directory = _directory(requirement_id, mode)
    relative = directory + "/progress.json"
    path = storage_path(project_root, relative)
    if not path.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "progress_not_saved", "No committed progress exists for this requirement and mode.")
    raw = read_progress_bytes(path)
    progress = validate_progress_contract(parse_json_contract(raw, source=relative))
    if render_json_contract(progress) != raw:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_not_canonical", "Stored progress is not canonical JSON.")
    if progress["requirement_id"] != requirement_id or progress["mode"] != mode:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_identity_mismatch", "Stored progress identifies another requirement or mode.")
    history = storage_path(project_root, f"{directory}/history/{progress['revision']}/progress.json")
    if read_progress_bytes(history) != raw:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_history_mismatch", "Current progress differs from its saved history.")
    return {
        "schema": "work-progress-read/v1", "status": "saved", "path": relative,
        "sha256": hashlib.sha256(raw).hexdigest(), "progress": progress,
    }


def preview_progress(
    project_root: Path, value: object, *, expected_revision: int,
) -> dict[str, Any]:
    progress = validate_progress_contract(value)
    if type(expected_revision) is not int or expected_revision < 0:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "The expected revision must be a nonnegative integer.")
    directory = _directory(progress["requirement_id"], progress["mode"])
    relative = directory + "/progress.json"
    path = storage_path(project_root, relative)
    previous = read_progress(project_root, progress["requirement_id"], progress["mode"]) if path.exists() else None
    current_revision = previous["progress"]["revision"] if previous else 0
    if current_revision != expected_revision or progress["revision"] != expected_revision + 1:
        raise WorkError(ExitCode.WORKFLOW_STATE, "progress_revision_conflict", "Read and review current progress before saving the next revision.")
    history = f"{directory}/history/{progress['revision']}"
    if storage_path(project_root, history).exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "progress_save_pending",
            "An existing uncommitted revision requires review; do not retry or overwrite it.",
            {"path": history, "committed_revision": current_revision},
        )
    binding = {
        "schema": "work-progress-save-request/v1", "path": relative,
        "expected_revision": expected_revision,
        "previous_sha256": previous["sha256"] if previous else None,
        "progress": progress,
    }
    return {
        "schema": "work-progress-preview/v1", "status": "valid", "path": relative,
        "expected_revision": expected_revision,
        "approved_sha256": hashlib.sha256(render_json_contract(binding)).hexdigest(),
        "progress": progress,
        "source_validation": "not_checked",
        "evidence_trust": "historical_context_only",
        "formal_readiness": "not_established",
    }


def prepare_progress(
    project_root: Path, value: object, *, requirement_id: str, mode: str, expected_revision: int,
) -> dict[str, Any]:
    """Derive storage metadata from a complete supplied discussion, without writing."""
    if type(expected_revision) is not int or expected_revision < 0:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "The expected revision must be a nonnegative integer.")
    _directory(requirement_id, mode)
    content = strict_keys(value, location="progress_prepare", required=set(FIELDS) - {
        "schema", "requirement_id", "mode", "revision", "status",
    })
    candidate = {
        "schema": "work-discussion-progress/v1", "requirement_id": requirement_id,
        "mode": mode, "revision": expected_revision + 1, "status": "discussion_only",
        **content,
    }
    preview = preview_progress(project_root, candidate, expected_revision=expected_revision)
    return {**preview, "schema": "work-progress-prepare/v1"}


def save_progress(
    project_root: Path, value: object, *, expected_revision: int, approved_sha256: str,
) -> dict[str, Any]:
    sha256(approved_sha256, location="approved_sha256")

    def checked_preview() -> dict[str, Any]:
        preview = preview_progress(project_root, value, expected_revision=expected_revision)
        if preview["approved_sha256"] != approved_sha256:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_approval_changed", "The progress content or saved baseline changed after review.")
        return preview

    preview = checked_preview()
    progress = preview["progress"]
    directory = _directory(progress["requirement_id"], progress["mode"])
    history = f"{directory}/history/{progress['revision']}"

    def prepare_raw() -> bytes:
        return render_json_contract(checked_preview()["progress"])

    raw = publish_progress_revision(
        project_root,
        directory=directory,
        history=history,
        current_path=preview["path"],
        prepare_raw=prepare_raw,
    )
    result = read_progress(project_root, progress["requirement_id"], progress["mode"])
    if result["sha256"] != hashlib.sha256(raw).hexdigest():
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_write_mismatch", "The committed progress changed during verification.")
    return {**result, "schema": "work-progress-save/v1", "approved_sha256": approved_sha256}
