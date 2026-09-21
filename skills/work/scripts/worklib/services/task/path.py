"""Resolve Task file references without reading filesystem state."""

from pathlib import Path
from typing import Any

from ...technical.infrastructure.work_paths import portable_path_identity, resolve_project_relative_path


def task_path_candidates(
    contract: dict[str, Any], project_root: Path
) -> dict[str, tuple[Path, ...]]:
    result: dict[str, list[Path]] = {}
    tasks = contract.get("tasks")
    if not isinstance(tasks, list):
        return {}
    for task in tasks:
        if not isinstance(task, dict) or not isinstance(task.get("files"), list):
            continue
        for item in task["files"]:
            if not isinstance(item, dict):
                continue
            for field in ("path", "source", "destination"):
                value = item.get(field)
                if not isinstance(value, str):
                    continue
                try:
                    _, resolved = resolve_project_relative_path(
                        project_root, value, field="task_file"
                    )
                except (TypeError, ValueError):
                    continue
                identity = portable_path_identity(resolved)
                result.setdefault(identity, []).append(resolved)
    return {identity: tuple(paths) for identity, paths in result.items()}


__all__ = ["task_path_candidates"]
