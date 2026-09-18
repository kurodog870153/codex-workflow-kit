"""Task Collection filesystem access without contract validation."""

from pathlib import Path

from ...technical.infrastructure.file_io import read_raw
from ...technical.infrastructure.work_paths import (
    resolve_project_relative_path,
    validate_task_collection_index_path,
)
from ...technical.infrastructure.task_collection import (
    read_task_index,
    read_task_item,
    task_path_existence,
    task_collection_item_names,
)


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

__all__ = ["read_project_task_source", "read_raw", "read_task_index", "read_task_item", "resolve_project_relative_path", "task_collection_item_names", "task_path_existence", "validate_task_collection_index_path"]
