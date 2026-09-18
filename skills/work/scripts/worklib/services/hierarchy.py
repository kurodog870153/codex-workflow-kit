from __future__ import annotations

import re
from pathlib import Path

from ..contracts.hierarchy import HierarchyContract
from ..models.common.errors import ExitCode, WorkError


NAME_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
WORK_DIRECTORIES = frozenset({"plan", "task", "execute"})
Hierarchy = HierarchyContract


def build_hierarchy(work_directory: str, selected_paths: list[str]) -> HierarchyContract:
    if work_directory not in WORK_DIRECTORIES:
        raise WorkError(ExitCode.CONTRACT, "invalid_work_directory", "The work directory must be plan, task, or execute.", {"work_directory": work_directory})
    selected: list[str] = []
    for path in selected_paths:
        if not isinstance(path, str) or not path:
            raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_path", "A selected hierarchy path is invalid.", {"path": path})
        parts = path.split("/")
        if "general" in parts or any(not NAME_PATTERN.fullmatch(part) for part in parts):
            raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_path", "A selected hierarchy path must contain lowercase kebab-case segments and cannot include general.", {"path": path})
        selected.append(path)
    if len(selected) != len(set(selected)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_hierarchy_path", "Selected hierarchy paths must be unique.", {"selected_paths": selected})
    for path in selected:
        descendants = [candidate for candidate in selected if candidate.startswith(f"{path}/")]
        if descendants:
            raise WorkError(ExitCode.CONTRACT, "redundant_hierarchy_path", "A selected hierarchy path cannot be an ancestor of another selected path.", {"path": path, "descendants": descendants})
    resolved = ["general"]
    for selected_path in selected:
        parts = selected_path.split("/")
        for depth in range(1, len(parts) + 1):
            path = "/".join(parts[:depth])
            if path not in resolved:
                resolved.append(path)
    required_candidates = {"general", *selected}
    return HierarchyContract(
        schema="work-hierarchy/v1", work_directory=work_directory,
        selected_paths=selected, resolved_paths=resolved,
        required_paths=[path for path in resolved if path in required_candidates],
        optional_paths=[path for path in resolved if path not in required_candidates],
    )


def build_hierarchy_selection_json(raw: bytes, *, skill_root: Path) -> dict[str, object]:
    from .hierarchy_selection import build_hierarchy_selection_json as operation
    return operation(raw, skill_root=skill_root)


def validate_hierarchy_selection_json(raw: bytes, *, skill_root: Path) -> dict[str, object]:
    from .hierarchy_selection import validate_hierarchy_selection_json as operation
    return operation(raw, skill_root=skill_root)
