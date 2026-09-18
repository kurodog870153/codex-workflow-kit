from __future__ import annotations

from pathlib import Path
from typing import Any

from .prepare import prepare_progress as _prepare_progress, preview_progress as _preview_progress
from .read import read_progress as _read_progress
from .save import save_progress as _save_progress
from .validation import validate_progress_contract, validate_sha256, validate_strict_keys


def read_progress(project_root: Path, requirement_id: str, mode: str) -> dict[str, Any]:
    return _read_progress(project_root, requirement_id, mode, validate_progress=validate_progress_contract)


def preview_progress(project_root: Path, value: object, *, expected_revision: int) -> dict[str, Any]:
    return _preview_progress(project_root, value, expected_revision=expected_revision, validate_progress=validate_progress_contract, read_progress=read_progress)


def prepare_progress(project_root: Path, value: object, *, requirement_id: str, mode: str, expected_revision: int) -> dict[str, Any]:
    return _prepare_progress(project_root, value, requirement_id=requirement_id, mode=mode, expected_revision=expected_revision, preview=preview_progress, validate_keys=validate_strict_keys)


def save_progress(project_root: Path, value: object, *, expected_revision: int, approved_sha256: str) -> dict[str, Any]:
    return _save_progress(project_root, value, expected_revision=expected_revision, approved_sha256=approved_sha256, preview=preview_progress, read_progress=read_progress, validate_sha256=validate_sha256)


__all__ = ["prepare_progress", "preview_progress", "read_progress", "save_progress", "validate_progress_contract"]
