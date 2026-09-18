from __future__ import annotations

from pathlib import Path
from typing import Any

from ...services.progress.prepare import prepare_progress as _prepare, preview_progress as _preview
from ...services.progress.read import read_progress as _read
from ...services.progress.save import save_progress as _save
from ...services.progress.validation import (
    parse_progress_request, validate_progress_contract, validate_sha256,
    validate_strict_keys,
)


def read_progress(project_root: Path, requirement_id: str, mode: str) -> dict[str, Any]:
    return _read(project_root, requirement_id, mode, validate_progress=validate_progress_contract)


def preview_progress(project_root: Path, raw: bytes, *, source: str, expected_revision: int) -> dict[str, Any]:
    value = parse_progress_request(raw, source=source)
    return _preview(project_root, value, expected_revision=expected_revision, validate_progress=validate_progress_contract, read_progress=read_progress)


def prepare_progress(project_root: Path, raw: bytes, *, source: str, requirement_id: str, mode: str, expected_revision: int) -> dict[str, Any]:
    value = parse_progress_request(raw, source=source)
    preview = lambda root, candidate, **kwargs: _preview(root, candidate, validate_progress=validate_progress_contract, read_progress=read_progress, **kwargs)
    return _prepare(project_root, value, requirement_id=requirement_id, mode=mode, expected_revision=expected_revision, preview=preview, validate_keys=validate_strict_keys)


def save_progress(project_root: Path, raw: bytes, *, source: str, expected_revision: int, approved_sha256: str) -> dict[str, Any]:
    value = parse_progress_request(raw, source=source)
    preview = lambda root, candidate, **kwargs: _preview(root, candidate, validate_progress=validate_progress_contract, read_progress=read_progress, **kwargs)
    return _save(project_root, value, expected_revision=expected_revision, approved_sha256=approved_sha256, preview=preview, read_progress=read_progress, validate_sha256=validate_sha256)


__all__ = ["prepare_progress", "preview_progress", "read_progress", "save_progress"]
