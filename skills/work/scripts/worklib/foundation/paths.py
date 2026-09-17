from __future__ import annotations

import re
import unicodedata
from pathlib import Path, PureWindowsPath

from .errors import ExitCode, WorkError


REQUIREMENT_ID_PATTERN = re.compile(r"^[a-z0-9._-]+$")
WORKFLOW_ID_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
TRANSACTION_ID_PATTERN = re.compile(r"^[0-9]{8}T[0-9]{6}Z-[0-9a-f]{8}$")
TASK_ITEM_ID_PATTERN = re.compile(r"^TASK-\d{3}$")
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


def validate_workflow_id(workflow_id: str) -> str:
    if not isinstance(workflow_id, str) or not WORKFLOW_ID_PATTERN.fullmatch(workflow_id):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_workflow_id",
            "The workflow ID must use lowercase alphanumeric words separated by hyphens.",
            {"workflow_id": workflow_id},
        )
    _validate_segment(workflow_id, field="workflow_id")
    return workflow_id


def validate_transaction_id(transaction_id: str) -> str:
    if not isinstance(transaction_id, str) or not TRANSACTION_ID_PATTERN.fullmatch(transaction_id):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_transaction_id",
            "The transaction ID must use UTC YYYYMMDDTHHMMSSZ followed by an eight-character lowercase hexadecimal suffix.",
            {"transaction_id": transaction_id},
        )
    _validate_segment(transaction_id, field="transaction_id")
    return transaction_id


def transaction_directory(
    project_root: Path,
    *,
    requirement_id: str | None,
    workflow_id: str,
    transaction_id: str,
) -> tuple[str, Path]:
    if requirement_id is None:
        owner = "pending"
    else:
        owner = validate_requirement_id(requirement_id)
        if owner == "pending":
            raise WorkError(
                ExitCode.CONTRACT,
                "reserved_transaction_owner",
                "The pending transaction owner is reserved for work without a requirement ID.",
                {"requirement_id": requirement_id},
            )
    workflow_id = validate_workflow_id(workflow_id)
    transaction_id = validate_transaction_id(transaction_id)
    return resolve_project_relative_path(
        project_root,
        f"outputs/work/transactions/{owner}/{workflow_id}/{transaction_id}",
        field="transaction_directory",
    )


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


def validate_execution_task_layout(
    project_root: Path, raw_task_directory: str
) -> Path:
    _, task_directory = resolve_project_relative_path(
        project_root, raw_task_directory, field="execution_task_directory"
    )
    legacy_files = sorted(path.name for path in task_directory.glob("ATTEMPT-*.json"))
    if legacy_files:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_legacy_layout_unsupported",
            "Flat Attempt and Correction files are unsupported; use "
            "<TASK-ID>/<ATTEMPT-ID>/attempt.json and its corrections/ directory. "
            "No files were changed.",
            {"path": raw_task_directory, "files": legacy_files},
        )
    return task_directory


def portable_path_identity(path: Path) -> str:
    return unicodedata.normalize("NFC", str(path)).casefold()


def default_artifact_paths(project_root: Path, requirement_id: str) -> dict[str, str]:
    requirement_id = validate_requirement_id(requirement_id)
    paths = {
        "plan": f"outputs/work/plans/{requirement_id}.json",
        "task": f"outputs/work/tasks/{requirement_id}/index.json",
        "execution": f"outputs/work/executions/{requirement_id}",
    }
    for field, value in paths.items():
        resolve_project_relative_path(project_root, value, field=field)
    return paths


def default_task_collection_artifact_paths(
    project_root: Path, requirement_id: str
) -> dict[str, str]:
    """Return the active TASK collection defaults."""
    requirement_id = validate_requirement_id(requirement_id)
    paths = {
        "plan": f"outputs/work/plans/{requirement_id}.json",
        "task": f"outputs/work/tasks/{requirement_id}/index.json",
        "execution": f"outputs/work/executions/{requirement_id}",
    }
    for field, value in paths.items():
        resolve_project_relative_path(project_root, value, field=field)
    return paths


def validate_task_collection_index_path(
    project_root: Path,
    requirement_id: str,
    raw_index_path: str,
) -> tuple[str, Path]:
    requirement_id = validate_requirement_id(requirement_id)
    normalized, resolved = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    index_path = Path(normalized)
    if index_path.name != "index.json" or index_path.parent.name != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_index_path_requirement_mismatch",
            "The TASK index path must end with <requirement-id>/index.json.",
            {"path": normalized, "requirement_id": requirement_id},
        )
    return normalized, resolved


def resolve_task_collection_item_path(
    project_root: Path,
    requirement_id: str,
    raw_index_path: str,
    task_id: str,
    raw_item_path: str,
) -> tuple[str, Path]:
    if not isinstance(task_id, str) or not TASK_ITEM_ID_PATTERN.fullmatch(task_id):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_task_item_id",
            "A TASK item ID must use TASK-NNN.",
            {"task_id": task_id},
        )
    expected = f"tasks/{task_id}.json"
    if raw_item_path != expected:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_item_path_mismatch",
            "A TASK item path must exactly match tasks/<TASK-ID>.json.",
            {"task_id": task_id, "expected": expected, "actual": raw_item_path},
        )
    normalized_index, _ = validate_task_collection_index_path(
        project_root, requirement_id, raw_index_path
    )
    collection_relative = normalized_index.rsplit("/", 1)[0]
    _, collection_path = resolve_project_relative_path(
        project_root, collection_relative, field="task_collection_directory"
    )
    _, item_path = resolve_project_relative_path(
        project_root,
        f"{collection_relative}/{raw_item_path}",
        field="task_item_path",
    )
    if not _is_within(item_path, collection_path):
        raise WorkError(
            ExitCode.CONTRACT,
            "task_item_path_escapes_collection",
            "The resolved TASK item path escapes the formal TASK directory.",
            {"task_id": task_id, "path": raw_item_path},
        )
    return raw_item_path, item_path


def validate_task_item_path_aliases(paths: dict[str, Path]) -> None:
    aliases: dict[str, str] = {}
    for task_id, path in paths.items():
        identity = portable_path_identity(path)
        if identity in aliases:
            raise WorkError(
                ExitCode.CONTRACT,
                "task_item_path_alias",
                "Two TASK items resolve to the same portable path identity.",
                {"first": aliases[identity], "second": task_id},
            )
        aliases[identity] = task_id


def validate_artifact_paths(
    project_root: Path,
    requirement_id: str,
    artifacts: object,
    *,
    actual_plan_path: str,
    allow_task_index: bool = False,
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
    if plan_path.suffix != ".json" or plan_path.stem != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "plan_path_requirement_mismatch",
            "The Plan artifact path must end with the requirement ID and .json.",
        )
    task_names = {"task.json", "index.json"} if allow_task_index else {"task.json"}
    if task_path.name not in task_names or task_path.parent.name != requirement_id:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_path_requirement_mismatch",
            "The TASK artifact path has the wrong requirement directory or filename.",
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
