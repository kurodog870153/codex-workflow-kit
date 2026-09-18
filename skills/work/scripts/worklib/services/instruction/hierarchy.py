from __future__ import annotations

from ...models.common.errors import ExitCode, WorkError
from ...models.hierarchy import HierarchyContract
from ...models.instruction import CrossModeInstructionCatalog, InstructionCatalog


def instruction_hierarchy_projection(catalog: InstructionCatalog, hierarchy: HierarchyContract, cross_mode_catalog: CrossModeInstructionCatalog | None = None) -> list[str] | None:
    mode = catalog.mode
    if mode == "plan":
        if not hierarchy.selected_paths:
            return None
        assert cross_mode_catalog is not None
        cross_mode_paths = set(cross_mode_catalog.paths)
        for hierarchy_path in hierarchy.selected_paths:
            if hierarchy_path not in cross_mode_paths:
                parent, separator, _ = hierarchy_path.rpartition("/")
                parent_path = parent if separator else "general"
                raise WorkError(ExitCode.CONTRACT, "instruction_hierarchy_path_missing", "A selected instruction hierarchy path does not exist in the catalog.", {"mode": "all", "path": hierarchy_path, "parent": parent_path, "valid_choices": list(cross_mode_catalog.children.get(parent_path, ()))})
        mode_paths = set(catalog.paths)
        candidates: list[str] = []
        for selected_path in hierarchy.selected_paths:
            parts = selected_path.split("/")
            projected = next(("/".join(parts[:depth]) for depth in range(len(parts), 0, -1) if "/".join(parts[:depth]) in mode_paths), None)
            if projected is not None and projected not in candidates:
                candidates.append(projected)
        return [path for path in candidates if not any(candidate.startswith(f"{path}/") for candidate in candidates)]
    catalog_paths = set(catalog.paths)
    for hierarchy_path in hierarchy.resolved_paths:
        if hierarchy_path not in catalog_paths:
            parent, separator, _ = hierarchy_path.rpartition("/")
            parent_path = parent if separator else "general"
            raise WorkError(ExitCode.CONTRACT, "instruction_hierarchy_path_missing", "A selected instruction hierarchy path does not exist in the catalog.", {"mode": mode, "path": hierarchy_path, "parent": parent_path, "valid_choices": list(catalog.children.get(parent_path, ()))})
    return None


__all__ = ["instruction_hierarchy_projection"]
