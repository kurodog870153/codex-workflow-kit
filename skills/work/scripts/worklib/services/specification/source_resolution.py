"""Locate trusted specification artifacts inside the managed work directory."""
from __future__ import annotations

import json
from pathlib import Path

from ...models.common.errors import ExitCode, WorkError


def resolve_plan_path(project_root: Path, requirement_id: str) -> str:
    work_root = project_root / "outputs" / "work"
    candidates: list[str] = []
    if work_root.is_dir():
        root = project_root.resolve()
        for path in work_root.rglob("*.json"):
            if not path.is_file() or not path.resolve().is_relative_to(root):
                continue
            try:
                document = json.loads(path.read_bytes())
            except (OSError, UnicodeError, ValueError):
                continue
            if not isinstance(document, dict) or document.get("requirement_id") != requirement_id:
                continue
            if document.get("schema") != "work-plan/v1":
                continue
            relative = path.relative_to(project_root).as_posix()
            artifacts = document.get("artifacts")
            if not isinstance(artifacts, dict) or artifacts.get("plan") != relative:
                raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "spec_plan_binding_invalid",
                                "A matching Plan has an invalid artifact binding.", {"path": relative})
            candidates.append(relative)
    if len(candidates) != 1:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "spec_plan_source_ambiguous" if candidates else "spec_plan_source_missing",
                        "Exactly one trusted Plan must match the requirement.", {"requirement_id": requirement_id, "candidates": sorted(candidates)})
    return candidates[0]


def resolve_repair_artifacts(project_root: Path, requirement_id: str) -> dict[str, str]:
    work_root = project_root / "outputs" / "work"
    bindings: list[dict[str, str]] = []
    execution_paths: list[str] = []
    if work_root.is_dir():
        root = project_root.resolve()
        for path in work_root.rglob("*.json"):
            if not path.is_file() or not path.resolve().is_relative_to(root):
                continue
            try:
                document = json.loads(path.read_bytes())
            except (OSError, UnicodeError, ValueError):
                continue
            if not isinstance(document, dict) or document.get("requirement_id") != requirement_id:
                continue
            relative = path.relative_to(project_root).as_posix()
            if document.get("schema") == "work-execution-index/v1":
                execution_paths.append(relative)
                continue
            identity = {"work-plan/v1": "plan", "work-task-index/v1": "task"}.get(
                document.get("schema") if isinstance(document.get("schema"), str) else None)
            if identity is None:
                continue
            artifacts = document.get("artifacts")
            if not isinstance(artifacts, dict) or set(artifacts) != {"plan", "task", "execution"} or artifacts.get(identity) != relative:
                raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_binding_invalid",
                                "A matching repair source has a conflicting artifact binding.", {"path": relative})
            bindings.append(artifacts)
    if not bindings or any(binding != bindings[0] for binding in bindings[1:]) or len(bindings) > 2:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_source_ambiguous",
                        "Repair sources must establish one consistent artifact binding.", {"requirement_id": requirement_id})
    expected_execution = bindings[0]["execution"] + "/index.json"
    if len(execution_paths) > 1 or any(path != expected_execution for path in execution_paths):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_execution_binding_conflict",
                        "The execution index conflicts with the source artifact binding.",
                        {"expected": expected_execution, "candidates": sorted(execution_paths)})
    return dict(bindings[0])


def resolve_reconciliation_attempt(project_root: Path, requirement_id: str,
                                   task_position: int, attempt_position: int) -> tuple[str, dict]:
    work_root = project_root / "outputs" / "work"
    candidates: list[tuple[str, dict]] = []
    if work_root.is_dir():
        root = project_root.resolve()
        for path in work_root.rglob("index.json"):
            if not path.is_file() or not path.resolve().is_relative_to(root):
                continue
            try:
                index = json.loads(path.read_bytes())
            except (OSError, UnicodeError, ValueError):
                continue
            if isinstance(index, dict) and index.get("schema") == "work-execution-index/v1" and index.get("requirement_id") == requirement_id:
                candidates.append((path.parent.relative_to(project_root).as_posix(), index))
    if len(candidates) != 1:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "reconciliation_execution_source_ambiguous",
                        "Exactly one execution index must match the requirement.", {"requirement_id": requirement_id})
    directory, index = candidates[0]
    tasks = index.get("tasks")
    if not isinstance(tasks, list) or task_position > len(tasks) or not isinstance(tasks[task_position - 1], dict):
        raise WorkError(ExitCode.CONTRACT, "reconciliation_task_position", "The TASK position does not exist.")
    row = tasks[task_position - 1]
    attempt_id = f"ATTEMPT-{attempt_position:03d}"
    if attempt_position > 999 or row.get("latest_attempt") != attempt_id:
        raise WorkError(ExitCode.CONTRACT, "reconciliation_attempt_position", "Select the latest recorded Attempt position.")
    task_id = row.get("id")
    if not isinstance(task_id, str) or not task_id.startswith("TASK-"):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "reconciliation_task_identity", "The execution TASK identity is invalid.")
    relative = f"{directory}/{task_id}/{attempt_id}/attempt.json"
    path = project_root / relative
    if not path.is_file() or not path.resolve().is_relative_to(project_root.resolve()):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "reconciliation_attempt_missing", "The selected Attempt is missing.")
    try:
        attempt = json.loads(path.read_bytes())
    except (OSError, UnicodeError, ValueError) as error:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "reconciliation_attempt_invalid", "The selected Attempt is unreadable.") from error
    if not isinstance(attempt, dict) or attempt.get("attempt_id") != attempt_id or attempt.get("task_id") != task_id or attempt.get("status") == "in_progress":
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "reconciliation_attempt_identity", "The selected Attempt is not a matching closed Attempt.")
    return relative, attempt
