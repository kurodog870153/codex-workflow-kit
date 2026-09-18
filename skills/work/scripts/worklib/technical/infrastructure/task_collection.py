"""Filesystem loaders for formal TASK collection contracts."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from .file_io import read_raw
from .json_contract import parse_json_contract
from .work_paths import (
    resolve_project_relative_path,
    resolve_task_collection_item_path,
)


def read_task_index(
    project_root: Path,
    raw_index_path: str,
) -> tuple[str, bytes, dict[str, Any]]:
    normalized, index_path = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    raw = read_raw(index_path)
    return normalized, raw, parse_json_contract(raw, source=str(index_path))


def read_task_item(
    project_root: Path,
    raw_index_path: str,
    reference: dict[str, Any],
    *,
    requirement_id: str,
) -> tuple[Path, bytes, dict[str, Any]]:
    task_id = reference["id"]
    _, path = resolve_task_collection_item_path(
        project_root,
        requirement_id,
        raw_index_path,
        task_id,
        reference["path"],
    )
    raw = read_raw(path)
    return path, raw, parse_json_contract(raw, source=str(path))


def task_path_existence(paths: dict[str, tuple[Path, ...]]) -> dict[str, bool]:
    return {
        identity: any(path.exists() for path in candidates)
        for identity, candidates in paths.items()
    }


def read_project_task_source(
    project_root: Path,
    relative_path: str,
    *,
    field: str,
) -> tuple[str, Path, bytes]:
    normalized, path = resolve_project_relative_path(
        project_root, relative_path, field=field
    )
    return normalized, path, read_raw(path)


def task_collection_item_names(
    project_root: Path,
    raw_index_path: str,
) -> set[str]:
    _, index_path = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    directory = index_path.parent / "tasks"
    try:
        return {
            path.name
            for path in directory.iterdir()
            if path.suffix == ".json" and path.is_file()
        } if directory.is_dir() else set()
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "task_collection_directory_read_failed",
            "The TASK item directory could not be inspected.",
            {"path": str(directory)},
        ) from error
