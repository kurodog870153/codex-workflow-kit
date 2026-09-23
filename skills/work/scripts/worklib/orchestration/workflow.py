"""Read-only workflow continuation orchestration."""

from __future__ import annotations

from pathlib import Path

from ..business_services.plan import parse_roots, validate_plan_file
from ..business_services.task import load_task_collection
from ..business_services.task.draft_status import task_draft_status
from ..business_services.workflow import (
    inspect_workflow_artifacts, load_latest_attempts,
    workflow_state as project_workflow_state,
)


def workflow_state(project_root: Path, requirement_id: str, *, plan_path: str | None = None,
                   user_config_root: str, skill_roots: list[str]) -> dict[str, object]:
    roots = parse_roots(skill_roots)
    artifacts, observed = inspect_workflow_artifacts(
        project_root, requirement_id, plan_path=plan_path,
    )
    exists = observed["exists"]
    plan_validation = None
    draft = None
    task_validation = None
    if exists["plan"]:
        plan_validation = validate_plan_file(
            project_root, user_config_root, artifacts["plan"], skill_roots=roots
        )
        if plan_validation["requirement_id"] != requirement_id:
            raise ValueError("The selected Plan does not match the requested requirement.")
        if exists["task"]:
            task_validation = load_task_collection(
                project_root, user_config_root, artifacts["task"], skill_roots=roots
            )
        else:
            draft = task_draft_status(project_root, requirement_id)
    work_skill_root = Path(__file__).resolve().parents[3]
    latest_attempts = load_latest_attempts(
        project_root, artifacts, observed["execution_index"]
    )
    return project_workflow_state(
        work_skill_root, requirement_id, artifacts, plan_validation=plan_validation, draft=draft,
        task_validation=task_validation, execution_index=observed["execution_index"],
        latest_attempts=latest_attempts,
    )
