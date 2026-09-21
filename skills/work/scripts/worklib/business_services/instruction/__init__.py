"""Instruction use cases."""

from pathlib import Path

from ...models.hierarchy import HierarchyContract
from ...models.instruction import InstructionCatalogContract, InstructionSelectionContract, InstructionSourceSet, InstructionsContract
from ...services.hierarchy.path import build_hierarchy
from ...services.instruction.catalog import build_cross_mode_instruction_catalog, build_instruction_catalog
from ...services.instruction.hierarchy import instruction_hierarchy_projection
from ...services.instruction.root import instruction_root
from ...services.instruction.selection import build_instruction_selection as _build_selection
from ...services.instruction.source import load_instruction_sources as _load_sources
from ...services.instruction.task_selection import build_task_document_instruction_selection as _build_task_selection, validate_task_document_instruction_selection as _validate_task_selection
from ...services.instruction.validation import parse_instruction_selection, validate_instruction_selection as _validate_selection
from ...services.instruction.work_selection import build_work_instruction_selection as _build_work_selection, parse_work_instruction_selection, validate_work_instruction_selection as _validate_work_selection


def resolve_instruction_hierarchy(skill_root: Path, mode: str, selected_paths: list[str]) -> HierarchyContract:
    catalog_value = build_instruction_catalog(skill_root, mode)
    hierarchy = build_hierarchy(mode, selected_paths)
    cross_mode = build_cross_mode_instruction_catalog(skill_root) if mode == "plan" and hierarchy.selected_paths else None
    projected_paths = instruction_hierarchy_projection(catalog_value, hierarchy, cross_mode)
    if mode != "plan" or projected_paths is None:
        return hierarchy
    projected = build_hierarchy(mode, projected_paths)
    return HierarchyContract(schema="work-hierarchy/v1", work_directory=mode, selected_paths=hierarchy.selected_paths, resolved_paths=projected.resolved_paths, required_paths=projected.required_paths, optional_paths=projected.optional_paths)


def load_instruction_sources(skill_root: Path, mode: str, selected_paths: list[str], reference_names: list[str] | None = None) -> InstructionSourceSet:
    return _load_sources(skill_root, mode, resolve_instruction_hierarchy(skill_root, mode, selected_paths), reference_names)


def build_instruction_selection(*, skill_root: Path, mode: str, selected_paths: list[str], reference_names: list[str] | None = None) -> dict[str, object]:
    return _build_selection(load_instruction_sources(skill_root, mode, selected_paths, reference_names))


def validate_instruction_selection(value: object, *, skill_root: Path, mode: str, location: str = "instruction_selection") -> InstructionSourceSet:
    parsed = parse_instruction_selection(value, location=location)
    current = load_instruction_sources(skill_root, mode, parsed["selected_paths"], parsed["references"])
    return _validate_selection(value, current, location=location)


def build_work_instruction_selection(*, skill_root: Path, mode: str, selected_paths: list[str], reference_names: list[str] | None = None) -> dict[str, object]:
    return _build_work_selection(load_instruction_sources(skill_root, mode, selected_paths, reference_names))


def validate_work_instruction_selection(value: object, *, skill_root: Path, mode: str, selected_paths: list[str], location: str = "work_instruction_selection") -> InstructionSourceSet:
    parsed = parse_work_instruction_selection(value, selected_paths=selected_paths, location=location)
    current = load_instruction_sources(skill_root, mode, parsed["selected_paths"], parsed["references"])
    return _validate_work_selection(value, current, selected_paths=selected_paths, location=location)


def _task_sources(task_selections: list[object], skill_root: Path) -> list[InstructionSourceSet]:
    if not isinstance(task_selections, list) or not task_selections:
        return []
    return [validate_instruction_selection(value, skill_root=skill_root, mode="task", location=f"tasks[{index}].instruction_selection") for index, value in enumerate(task_selections)]


def build_task_document_instruction_selection(task_selections: list[object], *, skill_root: Path) -> dict[str, object]:
    return _build_task_selection(_task_sources(task_selections, skill_root))


def validate_task_document_instruction_selection(value: object, task_selections: list[object], *, skill_root: Path, location: str = "instruction_selection") -> dict[str, object]:
    expected = _build_task_selection(_task_sources(task_selections, skill_root))
    return _validate_task_selection(value, expected, location=location)

def catalog(skill_root: Path, mode: str) -> dict[str, object]:
    value = (
        build_cross_mode_instruction_catalog(skill_root).as_dict()
        if mode == "all"
        else build_instruction_catalog(skill_root, mode).as_dict()
    )
    return InstructionCatalogContract.model_validate(value).to_canonical_dict()


def resolve(skill_root: Path, mode: str, paths: list[str]) -> dict[str, object]:
    return resolve_instruction_hierarchy(skill_root, mode, paths).as_dict()


def load(
    skill_root: Path,
    mode: str,
    paths: list[str],
    references: list[str],
) -> dict[str, object]:
    hierarchy = resolve_instruction_hierarchy(skill_root, mode, paths)
    value = _load_sources(skill_root, mode, hierarchy, references).as_dict()
    return InstructionsContract.model_validate(value).to_canonical_dict()


def select(
    skill_root: Path,
    mode: str,
    paths: list[str],
    references: list[str],
) -> dict[str, object]:
    value = {
        "schema": "work-instruction-selection/v1",
        "mode": mode,
        "instruction_selection": build_instruction_selection(
            skill_root=skill_root,
            mode=mode,
            selected_paths=paths,
            reference_names=references,
        ),
    }
    return InstructionSelectionContract.model_validate(value).to_canonical_dict()


__all__ = ["build_instruction_selection", "build_task_document_instruction_selection", "build_work_instruction_selection", "catalog", "instruction_root", "load", "load_instruction_sources", "resolve", "resolve_instruction_hierarchy", "select", "validate_instruction_selection", "validate_task_document_instruction_selection", "validate_work_instruction_selection"]
