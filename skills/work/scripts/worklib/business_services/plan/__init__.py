"""Plan use cases."""

from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.plan import TOP_OPTIONAL, TOP_REQUIRED, PlanSemanticRequestContract
from ...models.skill import SkillRoot
from ...services.hierarchy.fingerprint import hierarchy_selection_sha256
from ...services.hierarchy.path import build_hierarchy
from ...services.hierarchy.selection import build_hierarchy_selection_snapshot, parse_hierarchy_selection_request
from ...services.hierarchy.validation import validate_hierarchy_selection_snapshot
from ...services.instruction.catalog import build_cross_mode_instruction_catalog, build_instruction_catalog
from ...services.instruction.hierarchy import instruction_hierarchy_projection
from ...services.instruction.history import stored_selection
from ...services.instruction.source import load_instruction_sources
from ...services.instruction.validation import string_array
from ...services.instruction.work_selection import build_work_instruction_selection as build_work_selection
from ...services.instruction.work_selection import validate_work_instruction_selection
from ...services.plan import document
from ...services.plan.ordering import order_plan_contract
from ...services.plan.persistence import create_plan_exclusively
from ...services.plan.validation import validate_plan_contract as validate_plan_value
from ...services.skill_catalog import parse_skill_root, snapshot_catalog_skill
from ...services.skill_selection import validate_skill_roots
from ...services.skill_selection import build_skill_selection
from ...services.skill_selection import selection_sha256
from ...services.skill_selection import validate_skill_selection as validate_skill_selection_value


def _strict(value: object, *, location: str, required: set[str], optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    allowed = required | (optional or set())
    missing, unknown = sorted(required - set(value)), sorted(set(value) - allowed)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    return value


def _text(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(ExitCode.CONTRACT, "empty_text_value", "A non-empty string is required.", {"location": location})
    return value


def _hierarchy(value: object, work_root: Path) -> dict[str, object]:
    catalog = build_cross_mode_instruction_catalog(work_root).as_dict()
    decision, selected, entries, stored = validate_hierarchy_selection_snapshot(value, catalog)
    build_hierarchy("plan", selected)
    catalog_sha256 = catalog["catalog_sha256"]
    assert isinstance(catalog_sha256, str)
    if stored != hierarchy_selection_sha256(decision, selected, entries, catalog_sha256):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "hierarchy_selection_fingerprint_mismatch", "The hierarchy selection fingerprint does not match its contents.")
    assert isinstance(value, dict)
    return dict(value)


def _instruction_sources(work_root: Path, selected: list[str], references: list[str] | None = None):
    catalog = build_instruction_catalog(work_root, "plan")
    hierarchy = build_hierarchy("plan", selected)
    cross_mode = build_cross_mode_instruction_catalog(work_root) if hierarchy.selected_paths else None
    projected = instruction_hierarchy_projection(catalog, hierarchy, cross_mode)
    if projected is not None:
        resolved = build_hierarchy("plan", projected)
        hierarchy = type(hierarchy)(schema="work-hierarchy/v1", work_directory="plan", selected_paths=hierarchy.selected_paths, resolved_paths=resolved.resolved_paths, required_paths=resolved.required_paths, optional_paths=resolved.optional_paths)
    return load_instruction_sources(work_root, "plan", hierarchy, references)


def _skill_snapshots(value: object, roots: list[SkillRoot]) -> dict[tuple[str, str, str], dict[str, object]]:
    roots_by_identity = validate_skill_roots(roots)
    result: dict[tuple[str, str, str], dict[str, object]] = {}
    if not isinstance(value, dict) or not isinstance(value.get("skills"), list):
        return result
    for item in value["skills"]:
        if not isinstance(item, dict):
            continue
        scope, locator, source = item.get("scope"), item.get("root"), item.get("source")
        if not all(isinstance(part, str) and part.strip() for part in (scope, locator, source)):
            continue
        root = roots_by_identity.get((scope, locator))
        if root is not None:
            result[(scope, locator, source)] = snapshot_catalog_skill(root, source)
    return result


def render_plan_contract(contract: dict[str, Any]) -> bytes:
    return document.render(order_plan_contract(contract))


def validate_plan_contract(raw: bytes, *, source: str, actual_plan_path: str, project_root: Path,
                           user_config_root: str, skill_roots: list[SkillRoot] | None = None,
                           _historical_work_sources: bool = False, _allow_task_index: bool = False) -> dict[str, object]:
    del user_config_root
    contract = document.parse(raw, source=source)
    _strict(contract, location="plan", required=TOP_REQUIRED, optional=TOP_OPTIONAL)
    work_root = document.installed_work_root()
    hierarchy = _hierarchy(contract.get("hierarchy_selection"), work_root)
    selected = hierarchy["selected_paths"]
    assert isinstance(selected, list)
    if _historical_work_sources:
        instruction_sha = stored_selection(contract.get("work_instruction_selection"), selected_paths=selected)["instructions_sha256"]
    else:
        current = _instruction_sources(work_root, selected)
        instruction_sha = validate_work_instruction_selection(contract.get("work_instruction_selection"), current, selected_paths=selected).instructions_sha256
    roots = skill_roots or []
    skill_validation = validate_skill_selection_value(contract.get("skill_selection"), roots=roots, snapshots=_skill_snapshots(contract.get("skill_selection"), roots))
    skill = skill_validation["skill_selection"]
    assert isinstance(skill, dict) and isinstance(instruction_sha, str)
    return validate_plan_value(raw, source=source, actual_plan_path=actual_plan_path,
        project_root=project_root, hierarchy_selection=hierarchy,
        instruction_fingerprint=instruction_sha, skill_selection_sha256=skill["selection_sha256"],
        ordered_contract=order_plan_contract(contract), _allow_task_index=_allow_task_index)


def prepare_plan_json_contract(raw: bytes, *, source: str, actual_plan_path: str, project_root: Path,
                               user_config_root: str, skill_roots: list[SkillRoot] | None = None) -> tuple[dict[str, object], bytes]:
    contract = document.parse(raw, source=source)
    rendered = render_plan_contract(contract)
    validation = validate_plan_contract(rendered, source=source, actual_plan_path=actual_plan_path,
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        _allow_task_index=str(contract.get("artifacts", {}).get("task", "")).endswith("/index.json"))
    return validation, rendered


def validate_plan_json_contract(raw: bytes, **options) -> dict[str, object]:
    validation, _ = prepare_plan_json_contract(raw, **options)
    return validation


def validate_plan_request(*, input_file: bool, plan_path: str | None, path: str | None,
                          raw: bytes | None, source: str | None, project_root: Path,
                          user_config_root: str, skill_roots: list[SkillRoot]) -> dict[str, object]:
    if input_file:
        if not plan_path:
            raise WorkError(ExitCode.CLI_USAGE, "plan_path_required", "--plan-path is required with --input-file.")
        assert raw is not None and source is not None
        return validate_plan_json_contract(raw, source=source, actual_plan_path=plan_path,
            project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    if plan_path:
        raise WorkError(ExitCode.CLI_USAGE, "unexpected_plan_path", "--plan-path is only valid with --input-file.")
    assert path is not None
    return validate_plan_file(project_root, user_config_root, path, skill_roots=skill_roots)


def validate_plan_file(project_root: Path, user_config_root: str, raw_path: str, *, skill_roots: list[SkillRoot] | None = None) -> dict[str, object]:
    normalized, path = document.resolve_project_relative_path(project_root, raw_path, field="plan_path")
    raw = document.read(path)
    contract = document.parse(raw, source=str(path))
    return validate_plan_contract(raw, source=str(path), actual_plan_path=normalized,
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        _allow_task_index=str(contract.get("artifacts", {}).get("task", "")).endswith("/index.json"))


def build_semantic_selections(semantic: dict[str, Any], *, skill_roots: list[SkillRoot] | None = None) -> tuple[dict[str, object], dict[str, object]]:
    decision, selected, reasons = parse_hierarchy_selection_request(semantic["hierarchy_selection_request"])
    build_hierarchy("plan", selected)
    catalog = build_cross_mode_instruction_catalog(document.installed_work_root()).as_dict()
    hierarchy = build_hierarchy_selection_snapshot(decision, selected, reasons, catalog)
    entries, catalog_sha256 = hierarchy["entries"], hierarchy["catalog_sha256"]
    assert isinstance(entries, list) and isinstance(catalog_sha256, str)
    hierarchy["selection_sha256"] = hierarchy_selection_sha256(decision, selected, entries, catalog_sha256)
    roots = skill_roots or []
    request = semantic["skill_selection_request"]
    skill = build_skill_selection(request, roots=roots, snapshots=_skill_snapshots(request, roots))
    return hierarchy, skill


def _prepare_initial_plan(request: dict[str, Any], *, source: str, project_root: Path, user_config_root: str,
                          skill_roots: list[SkillRoot] | None = None,
                          output_file: str | None = None) -> dict[str, object]:
    requirement = _text(request["requirement_id"], location="requirement_id")
    content = request["content"]
    artifacts = document.default_artifact_paths(project_root, requirement)
    _, path = document.resolve_project_relative_path(project_root, artifacts["plan"], field="plan_path")
    if path.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "plan_already_exists", "Initial preparation cannot revise an existing Plan.")
    work_root = document.installed_work_root()
    hierarchy = _hierarchy(request["hierarchy_selection"], work_root)
    references = string_array(request["references"], location="references", allow_empty=True)
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Confirmed references must be unique.")
    instructions = build_work_selection(_instruction_sources(work_root, hierarchy["selected_paths"], references))
    candidate = {"schema": "work-plan/v1", "requirement_id": requirement, "status": "confirmed",
        **copy.deepcopy(content), "artifacts": artifacts, "hierarchy_selection": copy.deepcopy(hierarchy),
        "work_instruction_selection": instructions, "skill_selection": copy.deepcopy(request["skill_selection"])}
    validation, rendered = prepare_plan_json_contract(document.render(candidate), source=source,
        actual_plan_path=artifacts["plan"], project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    _, current = document.resolve_project_relative_path(project_root, artifacts["plan"], field="plan_path")
    if current != path or current.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "plan_prepare_target_changed", "The Plan target changed during preparation.")
    if output_file is not None:
        try:
            with Path(output_file).open("xb") as output:
                output.write(rendered)
        except FileExistsError as error:
            raise WorkError(
                ExitCode.WORKFLOW_STATE, "plan_prepare_output_exists",
                "The prepared Plan output file already exists.", {"path": output_file},
            ) from error
        except OSError as error:
            raise WorkError(
                ExitCode.IO_FAILURE, "plan_prepare_output_failed",
                "The prepared Plan output file could not be created.", {"path": output_file},
            ) from error
    return {"schema": "work-plan-prepare/v1", "status": "prepared", "path": artifacts["plan"],
            "plan": document.parse(rendered, source="prepared Plan"), "validation": validation}


def create_plan_file(raw: bytes, *, source: str, raw_plan_path: str, project_root: Path,
                     user_config_root: str, skill_roots: list[SkillRoot] | None = None) -> dict[str, object]:
    normalized, path = document.resolve_project_relative_path(project_root, raw_plan_path, field="plan_path")
    validation, rendered = prepare_plan_json_contract(raw, source=source, actual_plan_path=normalized,
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    if path.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "plan_already_exists", "The Plan target already exists; plan create never overwrites it.", {"path": normalized})
    create_plan_exclusively(path, rendered, normalized=normalized)
    stored = validate_plan_file(project_root, user_config_root, normalized, skill_roots=skill_roots)
    if stored != validation:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "plan_post_write_mismatch", "The stored Plan does not match the validated canonical Plan.", {"path": normalized})
    result = dict(stored)
    result.update(schema="work-plan-create/v1", path=normalized)
    return result


def prepare_semantic_plan(raw: bytes, *, source: str, project_root: Path, user_config_root: str,
                          skill_roots: list[SkillRoot] | None = None,
                          output_file: str | None = None) -> dict[str, object]:
    semantic = PlanSemanticRequestContract.parse_json_bytes(raw, source=source).to_canonical_dict()
    hierarchy, skill = build_semantic_selections(semantic, skill_roots=skill_roots)
    goal_ids = [f"GOAL-{index:03d}" for index in range(1, len(semantic["goals"]) + 1)]
    deliverable_ids = [f"DELIVERABLE-{index:03d}" for index in range(1, len(semantic["deliverables"]) + 1)]
    acceptance_ids = [f"ACCEPTANCE-{index:03d}" for index in range(1, len(semantic["acceptance_criteria"]) + 1)]
    content = {
        "title": semantic["title"], "summary": semantic["summary"],
        "goals": [{"id": item_id, "statement": statement} for item_id, statement in zip(goal_ids, semantic["goals"])],
        "scope": [{"id": f"SCOPE-{index:03d}", "kind": "in_scope", "statement": statement, "goal_ids": goal_ids}
                  for index, statement in enumerate(semantic["scope"], 1)],
        "deliverables": [{"id": item_id, "statement": statement, "goal_ids": goal_ids, "acceptance_ids": acceptance_ids}
                         for item_id, statement in zip(deliverable_ids, semantic["deliverables"])],
        "acceptance_criteria": [{"id": item_id, "statement": statement, "deliverable_ids": deliverable_ids}
                                for item_id, statement in zip(acceptance_ids, semantic["acceptance_criteria"])],
    }
    request = {"requirement_id": semantic["requirement_id"], "content": content,
               "hierarchy_selection": hierarchy,
               "skill_selection": skill,
               "references": semantic["references"]}
    return _prepare_initial_plan(request, source=source, project_root=project_root,
                                 user_config_root=user_config_root, skill_roots=skill_roots,
                                 output_file=output_file)


def parse_roots(values: list[str]) -> list[SkillRoot]:
    return [parse_skill_root(value) for value in values]


__all__ = ["create_plan_file", "parse_roots", "prepare_semantic_plan", "prepare_plan_json_contract", "render_plan_contract", "validate_plan_contract", "validate_plan_file", "validate_plan_json_contract", "validate_plan_request"]
