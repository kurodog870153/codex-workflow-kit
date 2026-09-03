from __future__ import annotations

import re
from dataclasses import dataclass

from .errors import ExitCode, WorkError


NAME_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
WORK_DIRECTORIES = frozenset({"plan", "task", "execute"})


@dataclass(frozen=True)
class Hierarchy:
    work_directory: str
    selected_paths: tuple[str, ...]
    resolved_paths: tuple[str, ...]
    required_paths: tuple[str, ...]
    optional_paths: tuple[str, ...]

    def as_dict(self) -> dict[str, object]:
        return {
            "schema": "work-hierarchy/v1",
            "work_directory": self.work_directory,
            "selected_paths": list(self.selected_paths),
            "resolved_paths": list(self.resolved_paths),
            "required_paths": list(self.required_paths),
            "optional_paths": list(self.optional_paths),
        }


def _validate_work_directory(work_directory: str) -> None:
    if work_directory not in WORK_DIRECTORIES:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_work_directory",
            "The work directory must be plan, task, or execute.",
            {"work_directory": work_directory},
        )


def _validate_selected_path(path: object) -> str:
    if not isinstance(path, str) or not path:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_path",
            "A selected hierarchy path is invalid.",
            {"path": path},
        )
    parts = path.split("/")
    if "general" in parts or any(not NAME_PATTERN.fullmatch(part) for part in parts):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_path",
            "A selected hierarchy path must contain lowercase kebab-case segments and cannot include general.",
            {"path": path},
        )
    return path


def build_hierarchy(work_directory: str, selected_paths: list[str]) -> Hierarchy:
    _validate_work_directory(work_directory)
    selected = [_validate_selected_path(path) for path in selected_paths]
    if len(selected) != len(set(selected)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_hierarchy_path",
            "Selected hierarchy paths must be unique.",
            {"selected_paths": selected},
        )
    for path in selected:
        descendants = [
            candidate for candidate in selected if candidate.startswith(f"{path}/")
        ]
        if descendants:
            raise WorkError(
                ExitCode.CONTRACT,
                "redundant_hierarchy_path",
                "A selected hierarchy path cannot be an ancestor of another selected path.",
                {"path": path, "descendants": descendants},
            )

    resolved = ["general"]
    for selected_path in selected:
        parts = selected_path.split("/")
        for depth in range(1, len(parts) + 1):
            path = "/".join(parts[:depth])
            if path not in resolved:
                resolved.append(path)

    required_candidates = {"general", *selected}
    required = [path for path in resolved if path in required_candidates]
    optional = [path for path in resolved if path not in required_candidates]
    return Hierarchy(
        work_directory=work_directory,
        selected_paths=tuple(selected),
        resolved_paths=tuple(resolved),
        required_paths=tuple(required),
        optional_paths=tuple(optional),
    )
