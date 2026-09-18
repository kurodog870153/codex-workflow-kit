from __future__ import annotations

import re
from pathlib import Path

from ...protocol import TASK_ID_PATTERN as TASK_ID_PATTERN_TEXT
from ...models.common.errors import ExitCode, WorkError
from ...models.common.identifiers import IdentifierPolicy
from .path_safety import (
    is_within as _is_within,
    normalize_relative_path,
    portable_path_identity,
    resolve_project_relative_path,
    resolve_root,
    validate_segment as _validate_segment,
)


TASK_ITEM_ID_PATTERN = re.compile(TASK_ID_PATTERN_TEXT)




def validate_execution_task_layout(
    project_root: Path, raw_task_directory: str
) -> Path:
    _, task_directory = resolve_project_relative_path(
        project_root, raw_task_directory, field="execution_task_directory"
    )
    legacy_files = sorted(path.name for path in task_directory.glob("ATTEMPT-*.json"))
    if legacy_files:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_legacy_layout_unsupported",
            "Flat Attempt and Correction files are unsupported; use "
            "<TASK-ID>/<ATTEMPT-ID>/attempt.json and its corrections/ directory. "
            "No files were changed.",
            {"path": raw_task_directory, "files": legacy_files},
        )
    return task_directory


def default_artifact_paths(project_root: Path, requirement_id: str) -> dict[str, str]:
    requirement_id = IdentifierPolicy.requirement_id(requirement_id)
    paths = {
        "plan": f"outputs/work/plans/{requirement_id}.json",
        "task": f"outputs/work/tasks/{requirement_id}/index.json",
        "execution": f"outputs/work/executions/{requirement_id}",
    }
    for field, value in paths.items():
        resolve_project_relative_path(project_root, value, field=field)
    return paths


def default_task_collection_artifact_paths(
    project_root: Path, requirement_id: str
) -> dict[str, str]:
    """Return the active TASK collection defaults."""
    requirement_id = IdentifierPolicy.requirement_id(requirement_id)
    paths = {
        "plan": f"outputs/work/plans/{requirement_id}.json",
        "task": f"outputs/work/tasks/{requirement_id}/index.json",
        "execution": f"outputs/work/executions/{requirement_id}",
    }
    for field, value in paths.items():
        resolve_project_relative_path(project_root, value, field=field)
    return paths


def validate_task_collection_index_path(
    project_root: Path,
    requirement_id: str,
    raw_index_path: str,
) -> tuple[str, Path]:
    requirement_id = IdentifierPolicy.requirement_id(requirement_id)
    normalized, resolved = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    index_path = Path(normalized)
    if index_path.name != "index.json" or index_path.parent.name != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_index_path_requirement_mismatch",
            "The TASK index path must end with <requirement-id>/index.json.",
            {"path": normalized, "requirement_id": requirement_id},
        )
    return normalized, resolved


def resolve_task_collection_item_path(
    project_root: Path,
    requirement_id: str,
    raw_index_path: str,
    task_id: str,
    raw_item_path: str,
) -> tuple[str, Path]:
    if not isinstance(task_id, str) or not TASK_ITEM_ID_PATTERN.fullmatch(task_id):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_task_item_id",
            "A TASK item ID must use TASK-NNN.",
            {"task_id": task_id},
        )
    expected = f"tasks/{task_id}.json"
    if raw_item_path != expected:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_item_path_mismatch",
            "A TASK item path must exactly match tasks/<TASK-ID>.json.",
            {"task_id": task_id, "expected": expected, "actual": raw_item_path},
        )
    normalized_index, _ = validate_task_collection_index_path(
        project_root, requirement_id, raw_index_path
    )
    collection_relative = normalized_index.rsplit("/", 1)[0]
    _, collection_path = resolve_project_relative_path(
        project_root, collection_relative, field="task_collection_directory"
    )
    _, item_path = resolve_project_relative_path(
        project_root,
        f"{collection_relative}/{raw_item_path}",
        field="task_item_path",
    )
    if not _is_within(item_path, collection_path):
        raise WorkError(
            ExitCode.CONTRACT,
            "task_item_path_escapes_collection",
            "The resolved TASK item path escapes the formal TASK directory.",
            {"task_id": task_id, "path": raw_item_path},
        )
    return raw_item_path, item_path


def validate_task_item_path_aliases(paths: dict[str, Path]) -> None:
    aliases: dict[str, str] = {}
    for task_id, path in paths.items():
        identity = portable_path_identity(path)
        if identity in aliases:
            raise WorkError(
                ExitCode.CONTRACT,
                "task_item_path_alias",
                "Two TASK items resolve to the same portable path identity.",
                {"first": aliases[identity], "second": task_id},
            )
        aliases[identity] = task_id


def validate_artifact_paths(
    project_root: Path,
    requirement_id: str,
    artifacts: object,
    *,
    actual_plan_path: str,
    allow_task_index: bool = False,
) -> dict[str, str]:
    requirement_id = IdentifierPolicy.requirement_id(requirement_id)
    if not isinstance(artifacts, dict) or set(artifacts) != {"plan", "task", "execution"}:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_artifact_paths",
            "Artifacts must contain exactly plan, task, and execution paths.",
        )

    normalized_paths: dict[str, str] = {}
    resolved_paths: dict[str, Path] = {}
    for field in ("plan", "task", "execution"):
        value = artifacts[field]
        if not isinstance(value, str):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_artifact_path",
                "Each artifact path must be a string.",
                {"field": field},
            )
        normalized, resolved = resolve_project_relative_path(
            project_root, value, field=f"artifacts.{field}"
        )
        normalized_paths[field] = normalized
        resolved_paths[field] = resolved

    plan_path = Path(normalized_paths["plan"])
    task_path = Path(normalized_paths["task"])
    execution_path = Path(normalized_paths["execution"])
    if plan_path.suffix != ".json" or plan_path.stem != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "plan_path_requirement_mismatch",
            "The Plan artifact path must end with the requirement ID and .json.",
        )
    task_names = {"task.json", "index.json"} if allow_task_index else {"task.json"}
    if task_path.name not in task_names or task_path.parent.name != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_path_requirement_mismatch",
            "The TASK artifact path has the wrong requirement directory or filename.",
        )
    if execution_path.name != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "execution_path_requirement_mismatch",
            "The execution artifact path must end with the requirement ID.",
        )

    normalized_actual = normalize_relative_path(actual_plan_path, field="actual_plan_path")
    if normalized_actual != normalized_paths["plan"]:
        raise WorkError(
            ExitCode.CONTRACT,
            "plan_artifact_path_mismatch",
            "The Plan contract path does not match the validated artifact path.",
            {"expected": normalized_paths["plan"], "actual": normalized_actual},
        )

    aliases: dict[str, str] = {}
    for field, path in resolved_paths.items():
        alias = portable_path_identity(path)
        if alias in aliases:
            raise WorkError(
                ExitCode.CONTRACT,
                "artifact_path_alias",
                "Two artifact paths resolve to the same portable path identity.",
                {"first": aliases[alias], "second": field},
            )
        aliases[alias] = field
    return normalized_paths
