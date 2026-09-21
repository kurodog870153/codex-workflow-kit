from pathlib import Path

from ...models.hierarchy import HierarchyContract
from ...models.instruction import InstructionSourceSet
from ...services.hierarchy.path import build_hierarchy
from ...services.instruction.catalog import build_instruction_catalog
from ...services.instruction.hierarchy import instruction_hierarchy_projection
from ...services.instruction.validation import validate_instruction_selection
from ...services.instruction.source import load_instruction_sources
from ...services.instruction.task_selection import (
    build_task_document_instruction_selection,
    validate_task_document_instruction_selection as validate_document_selection,
)
from ...services.instruction.validation import parse_instruction_selection


def _hierarchy(skill_root: Path, selected_paths: list[str]) -> HierarchyContract:
    catalog = build_instruction_catalog(skill_root, "task")
    hierarchy = build_hierarchy("task", selected_paths)
    instruction_hierarchy_projection(catalog, hierarchy)
    return hierarchy


def _sources(value: object, skill_root: Path, location: str) -> InstructionSourceSet:
    parsed = parse_instruction_selection(value, location=location)
    current = load_instruction_sources(
        skill_root,
        "task",
        _hierarchy(skill_root, parsed["selected_paths"]),
        parsed["references"],
    )
    return validate_instruction_selection(value, current, location=location)


def validate_task_document_selection(
    value: object,
    task_selections: list[object],
    *,
    skill_root: Path,
) -> dict[str, object]:
    sources = [
        _sources(selection, skill_root, f"tasks[{index}].instruction_selection")
        for index, selection in enumerate(task_selections)
    ]
    expected = build_task_document_instruction_selection(sources)
    return validate_document_selection(value, expected)


__all__ = ["validate_task_document_selection"]
