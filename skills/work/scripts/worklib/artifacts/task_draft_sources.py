"""Read-only source checks for a selected planning TASK."""

from __future__ import annotations

from pathlib import Path

from .task_draft import read_task_draft_from_index, read_task_planning_index
from ..contracts.plan import validate_plan_contract
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..foundation.runtime import installed_work_root
from ..hierarchy.selection import validate_task_hierarchy_paths
from ..instructions.selection import build_instruction_selection
from ..instructions.draft_selection import resolve_draft_instruction_selection
from ..skills.catalog import SkillRoot


def check_task_draft_sources(
    project_root: Path,
    requirement_id: str,
    task_id: str,
    *,
    expected_revision: int,
    plan_path: str,
    user_config_root: str,
    selected_paths: list[str] | None = None,
    reference_names: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    index = read_task_planning_index(project_root, requirement_id)
    if type(expected_revision) is not int or expected_revision != index["revision"]:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "Reload the current planning index before checking sources.")
    entry = next((task for task in index["tasks"] if task["id"] == task_id), None)
    if entry is None:
        raise WorkError(ExitCode.CONTRACT, "draft_task_not_in_index", "The selected TASK is absent from the planning index.")
    if "draft_ref" in entry:
        read_task_draft_from_index(project_root, index, task_id)
    selected = resolve_draft_instruction_selection(entry, selected_paths=selected_paths, reference_names=reference_names)
    normalized, resolved = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    raw = read_raw(resolved)
    validation = validate_plan_contract(
        raw, source=str(resolved), actual_plan_path=normalized,
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
    )
    if validation["requirement_id"] != requirement_id:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "draft_source_requirement_mismatch", "The Plan belongs to a different requirement.")
    mismatches = [field for field, stored in index["source"].items() if validation[field] != stored]
    if mismatches:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY, "draft_source_drift", "The saved planning sources differ from the validated Plan.",
            {"fields": sorted(mismatches), "task_id": task_id},
        )
    plan = parse_json_contract(raw, source=str(resolved))
    skill_id = entry["skill_id"]
    if skill_id is not None:
        skill = next((item for item in plan["skill_selection"]["skills"] if item["id"] == skill_id), None)
        if skill is None or skill["mode_support"]["task"] == "unsupported":
            raise WorkError(ExitCode.CONTRACT, "draft_skill_not_available", "The TASK skill is not a Plan-confirmed Task-capable skill.")
    work_root = installed_work_root()
    validate_task_hierarchy_paths(
        selected["selected_paths"], confirmed_selection=plan["hierarchy_selection"],
        skill_root=work_root, location="draft.instruction_paths",
    )
    selection = build_instruction_selection(
        skill_root=work_root, mode="task", selected_paths=selected["selected_paths"],
        reference_names=selected["references"],
    )
    if selection["instructions_sha256"] != entry["instructions_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "draft_instruction_drift", "The current TASK instruction fingerprint differs from the saved selection.", {"task_id": task_id})
    if read_task_planning_index(project_root, requirement_id) != index:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "The planning index changed while its sources were checked.")
    return {
        "schema": "work-task-draft-source-check/v1", "status": "valid",
        "requirement_id": requirement_id, "task_id": task_id, "revision": index["revision"],
        "source": dict(index["source"]), "skill_id": skill_id,
        "instructions_sha256": selection["instructions_sha256"],
        "instruction_selection": selected,
    }
