"""Plan context preparation for Task validation."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.plan import TOP_OPTIONAL, TOP_REQUIRED
from ...models.skill import SkillRoot
from ...services.hierarchy.fingerprint import hierarchy_selection_sha256
from ...services.hierarchy.path import build_hierarchy
from ...services.hierarchy.validation import validate_hierarchy_selection_snapshot
from ...services.instruction.catalog import build_cross_mode_instruction_catalog, build_instruction_catalog
from ...services.instruction.hierarchy import instruction_hierarchy_projection
from ...services.instruction.history import stored_selection
from ...services.instruction.source import load_instruction_sources
from ...services.instruction.work_selection import validate_work_instruction_selection
from ...services.plan import document
from ...services.plan.ordering import order_plan_contract
from ...services.plan.validation import validate_plan_contract as validate_plan_value
from ...services.skill_catalog import snapshot_catalog_skill
from ...services.skill_selection import validate_skill_roots
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


def validate_task_source_plan(raw: bytes, *, source: str, actual_plan_path: str, project_root: Path,
                           user_config_root: str, skill_roots: list[SkillRoot] | None = None,
                           _historical_work_sources: bool = False, _allow_task_index: bool = False,
                           _parsed_contract: dict[str, Any] | None = None) -> dict[str, object]:
    del user_config_root
    contract = _parsed_contract if _parsed_contract is not None else document.parse(raw, source=source)
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
        ordered_contract=order_plan_contract(contract), _allow_task_index=_allow_task_index,
        parsed_contract=contract)



__all__ = ["validate_task_source_plan"]
