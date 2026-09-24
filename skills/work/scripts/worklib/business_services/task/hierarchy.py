from pathlib import Path

from ...models.common.errors import ExitCode, WorkError
from ...services.hierarchy.path import build_hierarchy
from ...services.hierarchy.validation import validate_task_hierarchy_authorization
from ...services.instruction.catalog import build_instruction_catalog
from ...services.instruction.hierarchy import instruction_hierarchy_projection
from .validation_session import ValidationSession


def _resolve(skill_root: Path, mode: str, selected_paths: list[str],
             session: ValidationSession | None = None) -> None:
    catalog = session.catalog(mode) if session is not None else build_instruction_catalog(skill_root, mode)
    hierarchy = build_hierarchy(mode, selected_paths)
    projected = instruction_hierarchy_projection(catalog, hierarchy)
    if projected is not None:
        build_hierarchy(mode, projected)


def validate_task_hierarchy_paths(selected_paths: object, *, confirmed_selection: object,
                                  skill_root: Path, location: str,
                                  session: ValidationSession | None = None) -> tuple[str, ...]:
    if not isinstance(selected_paths, list) or any(not isinstance(path, str) for path in selected_paths):
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selected_paths",
                        "Hierarchy selected_paths must be an array of strings.", {"location": location})
    hierarchy = build_hierarchy("task", selected_paths)
    validate_task_hierarchy_authorization(hierarchy.selected_paths, confirmed_selection, location=location)
    for mode in ("task", "execute"):
        _resolve(skill_root, mode, list(hierarchy.selected_paths), session)
    return hierarchy.selected_paths


__all__ = ["validate_task_hierarchy_paths"]
