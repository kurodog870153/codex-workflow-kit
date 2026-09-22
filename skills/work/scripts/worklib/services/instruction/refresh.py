from __future__ import annotations

from pathlib import Path
from typing import Any

from ...technical.foundation.fingerprint import (
    canonical_json_sha256 as _canonical_json_sha256,
    raw_sha256 as _raw_sha256,
)
from ...technical.infrastructure.json_contract import parse_json_contract as _parse_json_contract
from ...technical.infrastructure.json_contract import render_json_contract as _render_json_contract
from ...technical.infrastructure.specification_storage import (
    replace_journal as _replace_journal,
    storage_path as _storage_path,
    write_exclusive as _write_exclusive,
)
from ...technical.infrastructure.text_codec import canonical_sha256 as _canonical_sha256
from ...technical.infrastructure.work_paths import default_artifact_paths as _default_artifact_paths


def canonical_json_sha256(value: object) -> str:
    return _canonical_json_sha256(value)


def raw_sha256(content: bytes) -> str:
    return _raw_sha256(content)


def parse_json_contract(content: bytes, *, source: str) -> dict[str, Any]:
    return _parse_json_contract(content, source=source)


def storage_path(project_root: Path, relative_path: str) -> Path:
    return _storage_path(project_root, relative_path)


def canonical_sha256(content: bytes, *, source: str) -> str:
    return _canonical_sha256(content, source=source)


def default_artifact_paths(project_root: Path, requirement_id: str) -> dict[str, str]:
    return _default_artifact_paths(project_root, requirement_id)


def render_json_contract(value: object) -> bytes:
    return _render_json_contract(value)


def write_exclusive(path: Path, raw: bytes) -> None:
    _write_exclusive(path, raw)


def replace_journal(path: Path, raw: bytes) -> None:
    _replace_journal(path, raw)


__all__ = [
    "canonical_json_sha256",
    "canonical_sha256",
    "default_artifact_paths",
    "parse_json_contract",
    "render_json_contract",
    "replace_journal",
    "raw_sha256",
    "storage_path",
    "write_exclusive",
]
