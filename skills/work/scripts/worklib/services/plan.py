from __future__ import annotations

import copy
from pathlib import Path

from ..contracts.plan import TOP_OPTIONAL
from ..infrastructure.plan_storage import create_plan_exclusively
from .plan_validation import prepare_plan_json_contract, validate_plan_file
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.runtime import installed_work_root
from ..services.hierarchy_selection import validate_hierarchy_selection
from .instruction_work_selection import build_work_instruction_selection
from .instruction_validation import string_array
from ..models.common.errors import ExitCode, WorkError
from ..foundation.paths import default_artifact_paths, resolve_project_relative_path, validate_artifact_paths
from .skill_catalog import SkillRoot


def prepare_initial_plan(
    raw: bytes, *, source: str, project_root: Path, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Prepare a validated initial candidate without publishing or granting approval."""
    request = strict_keys(parse_json_contract(raw, source=source), location="plan_prepare",
        required={"requirement_id", "content", "hierarchy_selection", "skill_selection", "references"},
        optional={"artifacts"})
    requirement = nonempty_string(request["requirement_id"], location="requirement_id")
    content = strict_keys(request["content"], location="content",
        required={"title", "summary", "goals", "scope", "deliverables", "acceptance_criteria"},
        optional=TOP_OPTIONAL - {"changes"})
    if "artifacts" in request:
        supplied = strict_keys(request["artifacts"], location="artifacts", required={"plan", "task", "execution"})
        artifacts = validate_artifact_paths(project_root, requirement, supplied, actual_plan_path=supplied["plan"])
    else:
        artifacts = default_artifact_paths(project_root, requirement)
    _, path = resolve_project_relative_path(project_root, artifacts["plan"], field="plan_path")
    if path.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "plan_already_exists", "Initial preparation cannot revise an existing Plan.")
    work_root = installed_work_root()
    hierarchy = validate_hierarchy_selection(request["hierarchy_selection"], skill_root=work_root)["hierarchy_selection"]
    references = string_array(request["references"], location="references", allow_empty=True)
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Confirmed references must be unique.")
    instructions = build_work_instruction_selection(skill_root=work_root, mode="plan",
        selected_paths=hierarchy["selected_paths"], reference_names=references)
    candidate = {
        "schema": "work-plan/v1", "requirement_id": requirement, "status": "confirmed",
        **copy.deepcopy(content), "artifacts": artifacts,
        "hierarchy_selection": copy.deepcopy(hierarchy), "work_instruction_selection": instructions,
        "skill_selection": copy.deepcopy(request["skill_selection"]),
    }
    validation, rendered = prepare_plan_json_contract(render_json_contract(candidate),
        source=source, actual_plan_path=artifacts["plan"], project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots)
    # Recheck target routing after source validation; creation still rechecks independently.
    _, current = resolve_project_relative_path(project_root, artifacts["plan"], field="plan_path")
    if current != path or current.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "plan_prepare_target_changed", "The Plan target changed during preparation.")
    return {"schema": "work-plan-prepare/v1", "status": "prepared", "path": artifacts["plan"],
            "plan": parse_json_contract(rendered, source="prepared Plan"), "validation": validation}


def create_plan_file(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    normalized, path = resolve_project_relative_path(
        project_root,
        raw_plan_path,
        field="plan_path",
    )
    validation, rendered = prepare_plan_json_contract(
        raw,
        source=source,
        actual_plan_path=normalized,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        )
    create_plan_exclusively(path, rendered, normalized=normalized)


    stored = validate_plan_file(
        project_root, user_config_root, normalized, skill_roots=skill_roots
    )
    if stored != validation:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "plan_post_write_mismatch",
            "The stored Plan does not match the validated canonical Plan.",
            {"path": normalized},
        )
    result = dict(stored)
    result["schema"] = "work-plan-create/v1"
    result["path"] = normalized
    return result
