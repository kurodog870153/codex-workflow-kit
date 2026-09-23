from __future__ import annotations

from pathlib import Path

from ...technical.infrastructure.file_io import read_raw
from ...technical.infrastructure.json_contract import parse_json_contract
from ...technical.infrastructure.path_safety import resolve_project_relative_path
from ...technical.infrastructure.work_paths import (
    default_artifact_paths, validate_artifact_paths,
)


def inspect_artifact_state(paths: dict[str, str], project_root: Path) -> dict[str, object]:
    resolved = {name: project_root / value for name, value in paths.items()}
    execution_index = resolved["execution"] / "index.json"
    return {
        "paths": resolved,
        "exists": {name: path.exists() for name, path in resolved.items()},
        "execution_index_path": execution_index,
        "execution_index_raw": read_raw(execution_index) if execution_index.is_file() else None,
    }


def inspect_requirement_state(
    project_root: Path, requirement_id: str, *, plan_path: str | None = None,
) -> tuple[dict[str, str], dict[str, object]]:
    artifacts = default_artifact_paths(project_root, requirement_id)
    if plan_path is not None:
        normalized, selected_plan = resolve_project_relative_path(
            project_root, plan_path, field="plan_path",
        )
        artifacts["plan"] = normalized
        if selected_plan.is_file():
            contract = parse_json_contract(read_raw(selected_plan), source=str(selected_plan))
            artifacts = validate_artifact_paths(
                project_root,
                requirement_id,
                contract.get("artifacts"),
                actual_plan_path=normalized,
                allow_task_index=str(contract.get("artifacts", {}).get("task", "")).endswith(
                    "/index.json"
                ),
            )
    observed = inspect_artifact_state(artifacts, project_root)
    raw = observed["execution_index_raw"]
    observed["execution_index"] = (
        parse_json_contract(raw, source=str(observed["execution_index_path"])) if raw is not None else None
    )
    return artifacts, observed


def resolve_operation_artifact_path(
    project_root: Path, raw_path: str, *, field: str
) -> Path:
    _, path = resolve_project_relative_path(project_root, raw_path, field=field)
    return path
