"""Save one complete discussion without caller-authored storage metadata."""

from __future__ import annotations

import copy
from pathlib import Path

from .task_draft import (
    read_task_planning_index,
    read_task_planning_revision,
    recover_task_planning,
    save_task_planning,
)
from .task_draft_sources import check_task_draft_sources
from ..contracts.validation import strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..instructions.draft_selection import resolve_draft_instruction_selection
from ..skills.catalog import SkillRoot


def save_task_draft_request(
    project_root: Path, requirement_id: str, task_id: str, request: object, *,
    expected_revision: int, plan_path: str, user_config_root: str,
    selected_paths: list[str] | None = None, reference_names: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None, recover: bool = False,
) -> dict[str, object]:
    """Derive a single-task save or reconstruct its original recovery proposal.

    The request replaces discussion content, never boundaries or sources.
    Recovery derives metadata from the original historical index, even if the
    save committed before interruption. Existing storage verifies exact bytes.
    Neither content status nor this request establishes user authorization.
    """
    payload = strict_keys(
        request, location="draft_save_request",
        required={"status", "notes", "confirmed_decisions", "tentative",
                  "open_questions", "next_discussion_point"},
        optional={"task_candidate"},
    )
    if type(expected_revision) is not int or expected_revision < 1:
        raise WorkError(ExitCode.WORKFLOW_STATE, "invalid_expected_revision", "An existing planning revision is required.")
    current = read_task_planning_index(project_root, requirement_id)
    allowed_revisions = {expected_revision, expected_revision + 1} if recover else {expected_revision}
    if current["revision"] not in allowed_revisions:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "Reload the current planning index before saving or recovering.")
    previous = read_task_planning_revision(project_root, requirement_id, expected_revision) if recover else current
    entry = next((task for task in previous["tasks"] if task["id"] == task_id), None)
    if entry is None:
        raise WorkError(ExitCode.CONTRACT, "draft_task_not_in_index", "The selected TASK is absent from the planning index.")

    selected = resolve_draft_instruction_selection(entry, selected_paths=selected_paths, reference_names=reference_names)
    checked = check_task_draft_sources(
        project_root, requirement_id, task_id, expected_revision=current["revision"],
        plan_path=plan_path, user_config_root=user_config_root,
        selected_paths=selected["selected_paths"], reference_names=selected["references"], skill_roots=skill_roots,
    )
    if checked["source"] != previous["source"] or checked["instructions_sha256"] != entry["instructions_sha256"] or checked["skill_id"] != entry["skill_id"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "draft_source_drift", "The original save sources no longer match the validated sources.")
    if read_task_planning_index(project_root, requirement_id) != current:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "The planning index changed while preparing the save.")

    proposed = copy.deepcopy(previous)
    proposed["revision"] = expected_revision + 1
    proposed["current_task_id"] = task_id
    target = next(task for task in proposed["tasks"] if task["id"] == task_id)
    target["status"] = payload["status"]
    target["instruction_selection"] = copy.deepcopy(checked["instruction_selection"])
    draft = {
        "schema": "work-task-draft/v1", "requirement_id": requirement_id,
        "task_id": task_id, "revision": entry.get("draft_ref", {}).get("revision", 0) + 1,
        "boundary_revision": entry["boundary_revision"],
        "source": copy.deepcopy(previous["source"]),
        "instructions_sha256": entry["instructions_sha256"], **copy.deepcopy(payload),
    }
    operation = recover_task_planning if recover else save_task_planning
    return operation(project_root, proposed, expected_revision=expected_revision, draft=draft)
