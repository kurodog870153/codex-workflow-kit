"""Hierarchy use cases."""

from pathlib import Path

from ...models.common.errors import ExitCode, WorkError
from ...services.hierarchy.fingerprint import hierarchy_selection_sha256
from ...services.hierarchy.path import build_hierarchy
from ...services.hierarchy.selection import build_hierarchy_selection_snapshot, parse_hierarchy_selection_json, parse_hierarchy_selection_request
from ...services.hierarchy.validation import validate_hierarchy_selection_snapshot, validate_task_hierarchy_authorization
from ...services.instruction.catalog import build_cross_mode_instruction_catalog, build_instruction_catalog
from ...services.instruction.hierarchy import instruction_hierarchy_projection


def _catalog(skill_root: Path) -> dict[str, object]:
    return build_cross_mode_instruction_catalog(skill_root).as_dict()


def _resolve_instruction_hierarchy(skill_root: Path, mode: str, selected_paths: list[str]) -> None:
    catalog = build_instruction_catalog(skill_root, mode)
    hierarchy = build_hierarchy(mode, selected_paths)
    cross_mode = build_cross_mode_instruction_catalog(skill_root) if mode == "plan" and hierarchy.selected_paths else None
    projected_paths = instruction_hierarchy_projection(catalog, hierarchy, cross_mode)
    if projected_paths is not None:
        build_hierarchy(mode, projected_paths)


def build_hierarchy_selection(value: object, *, skill_root: Path) -> dict[str, object]:
    decision, selected_paths, reasons = parse_hierarchy_selection_request(value)
    build_hierarchy("plan", selected_paths)
    selection = build_hierarchy_selection_snapshot(decision, selected_paths, reasons, _catalog(skill_root))
    entries = selection["entries"]
    catalog_sha256 = selection["catalog_sha256"]
    assert isinstance(entries, list) and isinstance(catalog_sha256, str)
    selection["selection_sha256"] = hierarchy_selection_sha256(decision, selected_paths, entries, catalog_sha256)
    return selection


def validate_hierarchy_selection(value: object, *, skill_root: Path) -> dict[str, object]:
    catalog = _catalog(skill_root)
    decision, selected_paths, entries, stored_sha256 = validate_hierarchy_selection_snapshot(value, catalog)
    build_hierarchy("plan", selected_paths)
    catalog_sha256 = catalog["catalog_sha256"]
    assert isinstance(catalog_sha256, str)
    if stored_sha256 != hierarchy_selection_sha256(decision, selected_paths, entries, catalog_sha256):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "hierarchy_selection_fingerprint_mismatch", "The hierarchy selection fingerprint does not match its contents.")
    return {"schema": "work-hierarchy-selection-validation/v1", "status": "valid", "hierarchy_selection": dict(value)}


def build_hierarchy_selection_json(raw: bytes, *, skill_root: Path) -> dict[str, object]:
    return build_hierarchy_selection(parse_hierarchy_selection_json(raw), skill_root=skill_root)


def validate_hierarchy_selection_json(raw: bytes, *, skill_root: Path) -> dict[str, object]:
    return validate_hierarchy_selection(parse_hierarchy_selection_json(raw), skill_root=skill_root)


def validate_task_hierarchy_paths(selected_paths: object, *, confirmed_selection: object, skill_root: Path, location: str) -> tuple[str, ...]:
    if not isinstance(selected_paths, list) or any(not isinstance(path, str) for path in selected_paths):
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selected_paths", "Hierarchy selected_paths must be an array of strings.", {"location": location})
    hierarchy = build_hierarchy("task", selected_paths)
    validate_task_hierarchy_authorization(hierarchy.selected_paths, confirmed_selection, location=location)
    for mode in ("task", "execute"):
        _resolve_instruction_hierarchy(skill_root, mode, list(hierarchy.selected_paths))
    return hierarchy.selected_paths


__all__ = [
    "build_hierarchy", "build_hierarchy_selection", "build_hierarchy_selection_json",
    "validate_hierarchy_selection", "validate_hierarchy_selection_json",
    "validate_task_hierarchy_paths",
]
