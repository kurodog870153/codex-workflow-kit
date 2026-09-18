from __future__ import annotations

from pathlib import Path
from typing import Any

from ...services.execution.context import (
    find_task_row,
    read_contract,
    resolve_project_relative_path,
    validate_execution_identity,
)


def load_lifecycle_task_context(
    *, project_root: Path, user_config_root: str, raw_task_path: str,
    task_id: str, skill_roots: object, operations,
) -> tuple[str, Path, dict[str, Any], dict[str, object]]:
    normalized, path = resolve_project_relative_path(project_root, raw_task_path, field="task_path")
    context = operations.load_task_execution_context(
        project_root, user_config_root, normalized, task_id, skill_roots=skill_roots,
    )
    contract, validation = context["contract"], context["validation"]
    assert isinstance(contract, dict) and isinstance(validation, dict)
    return normalized, path, contract, validation


__all__ = ["find_task_row", "load_lifecycle_task_context", "read_contract", "validate_execution_identity"]
