from __future__ import annotations

import re
import unicodedata
from pathlib import Path, PureWindowsPath

from .errors import ExitCode, WorkError


REQUIREMENT_ID_PATTERN = re.compile(r"^[a-z0-9._-]+$")
WINDOWS_DEVICES = {
    "CON",
    "PRN",
    "AUX",
    "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}
UNSAFE_SEGMENT_CHARACTERS = frozenset('<>:"|?*')


def resolve_root(raw_path: str, *, label: str) -> Path:
    try:
        path = Path(raw_path).expanduser().resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "root_resolution_failed",
            f"The {label} could not be resolved.",
            {"path": raw_path},
        ) from error

    if not path.is_dir():
        raise WorkError(
            ExitCode.IO_FAILURE,
            "root_not_directory",
            f"The {label} is not a directory.",
            {"path": str(path)},
        )
    return path


def _is_windows_device(segment: str) -> bool:
    return segment.split(".", 1)[0].upper() in WINDOWS_DEVICES


def _validate_segment(segment: str, *, field: str) -> None:
    if not segment or segment in {".", ".."}:
        raise WorkError(
            ExitCode.CONTRACT,
            "unsafe_path_segment",
            "The path contains an empty, current, or parent segment.",
            {"field": field, "segment": segment},
        )
    if segment[-1] in {" ", "."}:
        raise WorkError(
            ExitCode.CONTRACT,
            "unsafe_path_segment",
            "A path segment cannot end with a space or dot.",
            {"field": field, "segment": segment},
        )
    if any(ord(character) < 32 for character in segment) or any(
        character in UNSAFE_SEGMENT_CHARACTERS for character in segment
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "unsafe_path_segment",
            "The path contains a character that is unsafe across supported platforms.",
            {"field": field, "segment": segment},
        )
    if _is_windows_device(segment):
        raise WorkError(
            ExitCode.CONTRACT,
            "windows_device_name",
            "The path contains a reserved Windows device name.",
            {"field": field, "segment": segment},
        )


def validate_requirement_id(requirement_id: str) -> str:
    if not REQUIREMENT_ID_PATTERN.fullmatch(requirement_id):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_requirement_id",
            "The requirement ID must use lowercase letters, digits, dots, underscores, or hyphens.",
            {"requirement_id": requirement_id},
        )
    _validate_segment(requirement_id, field="requirement_id")
    return requirement_id


def normalize_relative_path(raw_path: str, *, field: str = "path") -> str:
    if not isinstance(raw_path, str) or not raw_path:
        raise WorkError(
            ExitCode.CONTRACT,
            "empty_relative_path",
            "The project-relative path cannot be empty.",
            {"field": field},
        )

    windows_path = PureWindowsPath(raw_path)
    if windows_path.is_absolute() or windows_path.drive or raw_path.startswith(("/", "\\")):
        raise WorkError(
            ExitCode.CONTRACT,
            "absolute_path_rejected",
            "The path must be project-relative.",
            {"field": field, "path": raw_path},
        )

    value = raw_path
    if value.startswith(("./", ".\\")):
        value = value[2:]
    value = value.replace("\\", "/")
    segments = value.split("/")
    for segment in segments:
        _validate_segment(segment, field=field)
    return "/".join(segments)


def _is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def resolve_project_relative_path(
    project_root: Path,
    raw_path: str,
    *,
    field: str = "path",
) -> tuple[str, Path]:
    normalized = normalize_relative_path(raw_path, field=field)
    candidate = project_root.joinpath(*normalized.split("/"))

    unresolved_segments: list[str] = []
    nearest = candidate
    while not nearest.exists() and nearest != project_root:
        unresolved_segments.append(nearest.name)
        nearest = nearest.parent

    try:
        resolved_nearest = nearest.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "path_resolution_failed",
            "The path or its nearest existing ancestor could not be resolved.",
            {"field": field, "path": normalized},
        ) from error

    if not _is_within(resolved_nearest, project_root):
        raise WorkError(
            ExitCode.CONTRACT,
            "path_escapes_project_root",
            "The resolved path escapes the project root.",
            {"field": field, "path": normalized},
        )

    resolved_candidate = resolved_nearest.joinpath(*reversed(unresolved_segments))
    return normalized, resolved_candidate


def portable_path_identity(path: Path) -> str:
    return unicodedata.normalize("NFC", str(path)).casefold()


def default_artifact_paths(project_root: Path, requirement_id: str) -> dict[str, str]:
    requirement_id = validate_requirement_id(requirement_id)
    paths = {
        "plan": f"outputs/work/plans/{requirement_id}.md",
        "task": f"outputs/work/tasks/{requirement_id}.md",
        "execution": f"outputs/work/executions/{requirement_id}",
    }
    for field, value in paths.items():
        resolve_project_relative_path(project_root, value, field=field)
    return paths


def validate_artifact_paths(
    project_root: Path,
    requirement_id: str,
    artifacts: object,
    *,
    actual_plan_path: str,
) -> dict[str, str]:
    requirement_id = validate_requirement_id(requirement_id)
    if not isinstance(artifacts, dict) or set(artifacts) != {"plan", "task", "execution"}:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_artifact_paths",
            "Artifacts must contain exactly plan, task, and execution paths.",
        )

    normalized_paths: dict[str, str] = {}
    resolved_paths: dict[str, Path] = {}
    for field in ("plan", "task", "execution"):
        value = artifacts[field]
        if not isinstance(value, str):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_artifact_path",
                "Each artifact path must be a string.",
                {"field": field},
            )
        normalized, resolved = resolve_project_relative_path(
            project_root, value, field=f"artifacts.{field}"
        )
        normalized_paths[field] = normalized
        resolved_paths[field] = resolved

    plan_path = Path(normalized_paths["plan"])
    task_path = Path(normalized_paths["task"])
    execution_path = Path(normalized_paths["execution"])
    if plan_path.suffix != ".md" or plan_path.stem != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "plan_path_requirement_mismatch",
            "The Plan artifact path must end with the requirement ID and .md.",
        )
    if task_path.suffix != ".md" or task_path.stem != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_path_requirement_mismatch",
            "The TASK artifact path must end with the requirement ID and .md.",
        )
    if execution_path.name != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "execution_path_requirement_mismatch",
            "The execution artifact path must end with the requirement ID.",
        )

    normalized_actual = normalize_relative_path(actual_plan_path, field="actual_plan_path")
    if normalized_actual != normalized_paths["plan"]:
        raise WorkError(
            ExitCode.CONTRACT,
            "plan_artifact_path_mismatch",
            "The Plan contract path does not match the validated artifact path.",
            {"expected": normalized_paths["plan"], "actual": normalized_actual},
        )

    aliases: dict[str, str] = {}
    for field, path in resolved_paths.items():
        alias = portable_path_identity(path)
        if alias in aliases:
            raise WorkError(
                ExitCode.CONTRACT,
                "artifact_path_alias",
                "Two artifact paths resolve to the same portable path identity.",
                {"first": aliases[alias], "second": field},
            )
        aliases[alias] = field
    return normalized_paths
