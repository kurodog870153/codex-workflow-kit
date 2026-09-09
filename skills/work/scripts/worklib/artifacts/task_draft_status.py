"""Read-only planning progress and deterministic next discussion actions."""

from __future__ import annotations

import copy
from pathlib import Path

from .task_draft import _path, read_task_draft_from_index, read_task_planning_index
from ..contracts.task_draft import PLANNING_STATUSES
from ..foundation.errors import ExitCode, WorkError


def _exists(path: Path) -> bool:
    """Distinguish absence from an unreadable storage location."""
    try:
        path.stat()
    except FileNotFoundError:
        return False
    except OSError as error:
        raise WorkError(ExitCode.IO_FAILURE, "draft_status_read_failed", "Planning storage could not be inspected.") from error
    return True


def task_draft_status(
    project_root: Path, requirement_id: str, *, task_id: str | None = None,
) -> dict[str, object]:
    """Inspect committed progress without loading skills or proving live readiness.

    Prefer the saved current TASK, never automatically advance to another TASK.
    Only its historical discussion is read; assembly validates all candidates.
    Every returned action is advisory and preserves the workflow's approval gates.
    """
    index_path = _path(project_root, requirement_id, "index.json")
    result = {
        "schema": "work-task-draft-status/v1", "requirement_id": requirement_id,
        "status": "not_initialized", "revision": None, "current_task_id": None,
        "selected_task_id": None, "counts": {status: 0 for status in PLANNING_STATUSES},
        "tasks": [], "discussion": None, "next_action": "confirm_task_list",
        "required_checks": ["plan validate"], "requires_user_confirmation": True,
        "source_validation": "not_checked", "assembly_validation": "not_performed",
        "instruction_selection": None, "selection_confirmation_required": False,
    }
    if not _exists(index_path):
        try:
            residue = _exists(index_path.parent) and any(index_path.parent.iterdir())
        except OSError as error:
            raise WorkError(ExitCode.IO_FAILURE, "draft_status_read_failed", "Planning storage could not be inspected.") from error
        if _exists(index_path):
            raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "Planning was initialized during inspection; reload progress.")
        if residue:
            result.update(status="recovery_required", next_action="inspect_recovery", required_checks=[])
        elif task_id is not None:
            raise WorkError(ExitCode.CONTRACT, "draft_task_not_in_index", "No planning index exists for the selected TASK.")
        return result

    index = read_task_planning_index(project_root, requirement_id)
    selected_id = task_id if task_id is not None else index["current_task_id"]
    selected = next((entry for entry in index["tasks"] if entry["id"] == selected_id), None)
    if task_id is not None and selected is None:
        raise WorkError(ExitCode.CONTRACT, "draft_task_not_in_index", "The selected TASK is absent from the planning index.")
    counts = {status: sum(entry["status"] == status for entry in index["tasks"]) for status in PLANNING_STATUSES}
    result.update(
        status="saved", revision=index["revision"], current_task_id=index["current_task_id"],
        selected_task_id=selected_id, counts=counts, tasks=copy.deepcopy(index["tasks"]),
        instruction_selection=copy.deepcopy(selected.get("instruction_selection")) if selected else None,
        selection_confirmation_required=selected is not None and "instruction_selection" not in selected,
    )
    reserved = _path(project_root, requirement_id, f"history/{index['revision'] + 1}")
    if _exists(reserved):
        result.update(status="recovery_required", next_action="inspect_recovery", required_checks=[])
    else:
        if selected is not None and "draft_ref" in selected:
            draft = read_task_draft_from_index(project_root, index, selected_id)
            result["discussion"] = {
                key: copy.deepcopy(draft[key]) for key in (
                    "task_id", "revision", "status", "notes", "confirmed_decisions",
                    "tentative", "open_questions", "next_discussion_point",
                )
            }
            result["discussion"]["has_task_candidate"] = "task_candidate" in draft
        if counts["refined"] == len(index["tasks"]):
            result.update(next_action="assemble_for_review", required_checks=["task draft-assemble"])
        elif selected is None or selected["status"] == "refined":
            result.update(next_action="choose_task", required_checks=["plan validate", "task draft-check"])
        else:
            actions = {
                "planned": "confirm_start", "in_progress": "confirm_resume",
                "needs_review": "confirm_review",
            }
            result.update(next_action=actions[selected["status"]], required_checks=["plan validate", "task draft-check"])
    if read_task_planning_index(project_root, requirement_id) != index:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "The planning index changed during status inspection.")
    return result
