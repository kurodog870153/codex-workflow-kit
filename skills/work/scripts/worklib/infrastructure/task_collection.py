"""Filesystem loaders for formal TASK collection contracts."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..contracts.task_index import validate_task_index_contract
from ..contracts.task_item import validate_task_item_contract
from ..models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import (
    resolve_project_relative_path,
    resolve_task_collection_item_path,
)


def load_task_index(
    project_root: Path,
    raw_index_path: str,
) -> tuple[bytes, dict[str, Any], dict[str, object]]:
    normalized, index_path = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    raw = read_raw(index_path)
    contract = parse_json_contract(raw, source=str(index_path))
    requirement_id = contract.get("requirement_id")
    validation = validate_task_index_contract(
        raw,
        source=str(index_path),
        actual_index_path=normalized,
        project_root=project_root,
    )
    if validation["requirement_id"] != requirement_id:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_index_identity_mismatch",
            "The TASK index identity changed during validation.",
        )
    return raw, contract, validation


def load_task_item(
    project_root: Path,
    raw_index_path: str,
    reference: dict[str, Any],
    *,
    requirement_id: str,
) -> tuple[bytes, dict[str, Any], dict[str, object]]:
    task_id = reference["id"]
    _, path = resolve_task_collection_item_path(
        project_root,
        requirement_id,
        raw_index_path,
        task_id,
        reference["path"],
    )
    raw = read_raw(path)
    validation = validate_task_item_contract(
        raw, source=str(path), expected_task_id=task_id
    )
    if validation["task_item_sha256"] != reference["canonical_sha256"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_item_fingerprint_mismatch",
            "A TASK item fingerprint does not match the formal index.",
            {"task_id": task_id},
        )
    return raw, parse_json_contract(raw, source=str(path)), validation
