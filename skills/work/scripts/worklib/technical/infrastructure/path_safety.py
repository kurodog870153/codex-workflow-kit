"""Cross-platform path normalization and filesystem resolution adapters."""

from pathlib import Path, PureWindowsPath
import unicodedata

from ...models.common.errors import ExitCode, WorkError
from ...models.common.path_segment import PathSegmentPolicy


WINDOWS_DEVICES = {
    "CON", "PRN", "AUX", "NUL",
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


def validate_segment(segment: str, *, field: str) -> None:
    issue = PathSegmentPolicy.issue(segment)
    if issue == "unsafe_path_segment":
        raise WorkError(ExitCode.CONTRACT, "unsafe_path_segment", "The path contains an empty, current, or parent segment.", {"field": field, "segment": segment})
    if issue == "windows_device_name":
        raise WorkError(ExitCode.CONTRACT, "windows_device_name", "The path contains a reserved Windows device name.", {"field": field, "segment": segment})


def normalize_relative_path(raw_path: str, *, field: str = "path") -> str:
    if not isinstance(raw_path, str) or not raw_path:
        raise WorkError(ExitCode.CONTRACT, "empty_relative_path", "The project-relative path cannot be empty.", {"field": field})
    windows_path = PureWindowsPath(raw_path)
    if windows_path.is_absolute() or windows_path.drive or raw_path.startswith(("/", "\\")):
        raise WorkError(ExitCode.CONTRACT, "absolute_path_rejected", "The path must be project-relative.", {"field": field, "path": raw_path})
    value = raw_path
    if value.startswith(("./", ".\\")):
        value = value[2:]
    value = value.replace("\\", "/")
    for segment in value.split("/"):
        validate_segment(segment, field=field)
    return value


def is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def resolve_project_relative_path(project_root: Path, raw_path: str, *, field: str = "path") -> tuple[str, Path]:
    normalized = normalize_relative_path(raw_path, field=field)
    try:
        resolved_root = project_root.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise WorkError(ExitCode.IO_FAILURE, "path_resolution_failed", "The project root could not be resolved.", {"field": field, "path": normalized}) from error
    candidate = resolved_root.joinpath(*normalized.split("/"))
    unresolved_segments: list[str] = []
    nearest = candidate
    while not nearest.exists() and nearest != resolved_root:
        unresolved_segments.append(nearest.name)
        nearest = nearest.parent
    try:
        resolved_nearest = nearest.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise WorkError(ExitCode.IO_FAILURE, "path_resolution_failed", "The path or its nearest existing ancestor could not be resolved.", {"field": field, "path": normalized}) from error
    if not is_within(resolved_nearest, resolved_root):
        raise WorkError(ExitCode.CONTRACT, "path_escapes_project_root", "The resolved path escapes the project root.", {"field": field, "path": normalized})
    return normalized, resolved_nearest.joinpath(*reversed(unresolved_segments))


def portable_path_identity(path: Path) -> str:
    return unicodedata.normalize("NFC", str(path)).casefold()


__all__ = ["is_within", "normalize_relative_path", "portable_path_identity", "resolve_project_relative_path", "resolve_root", "validate_segment"]
