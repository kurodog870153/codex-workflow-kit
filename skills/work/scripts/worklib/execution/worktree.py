from __future__ import annotations

import json
import subprocess
from pathlib import Path, PurePosixPath
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from .preflight import execute_preflight
from ..foundation.fingerprint import canonical_sha256, read_raw
from ..foundation.markdown import parse_markdown_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot


GIT_TIMEOUT_SECONDS = 30


def _decode_git_path(raw: bytes) -> str:
    try:
        path = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "execute_worktree_invalid_utf8_path",
            "Git returned a path that is not valid UTF-8.",
        ) from error
    pure = PurePosixPath(path)
    if not path or pure.is_absolute() or any(part in {"", ".", ".."} for part in pure.parts):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "execute_worktree_invalid_path",
            "Git returned an invalid project-relative path.",
            {"path": path},
        )
    return path


def parse_porcelain_v1_z(raw: bytes) -> list[dict[str, str]]:
    parts = raw.split(b"\0")
    if parts and parts[-1] == b"":
        parts.pop()
    records: list[dict[str, str]] = []
    position = 0
    while position < len(parts):
        record = parts[position]
        if len(record) < 4 or record[2:3] != b" ":
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "execute_worktree_invalid_porcelain",
                "Git returned malformed porcelain v1 data.",
            )
        try:
            index_status = record[0:1].decode("ascii")
            worktree_status = record[1:2].decode("ascii")
        except UnicodeDecodeError as error:
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "execute_worktree_invalid_porcelain",
                "Git returned a non-ASCII porcelain status.",
            ) from error
        item = {
            "index_status": index_status,
            "worktree_status": worktree_status,
            "path": _decode_git_path(record[3:]),
        }
        position += 1
        if index_status in {"R", "C"} or worktree_status in {"R", "C"}:
            if position >= len(parts):
                raise WorkError(
                    ExitCode.INPUT_FORMAT,
                    "execute_worktree_invalid_porcelain",
                    "A Git rename or copy record is incomplete.",
                )
            item["original_path"] = _decode_git_path(parts[position])
            position += 1
        records.append(item)
    return records


def _run_git(project_root: Path, arguments: list[str]) -> bytes:
    try:
        result = subprocess.run(
            ["git", *arguments],
            cwd=project_root,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=GIT_TIMEOUT_SECONDS,
        )
    except FileNotFoundError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execute_worktree_git_missing",
            "Git is not available.",
        ) from error
    except subprocess.TimeoutExpired as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execute_worktree_git_timeout",
            "The read-only Git command timed out.",
        ) from error
    if result.returncode != 0:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execute_worktree_git_failed",
            "The read-only Git command failed.",
            {"exit_code": result.returncode},
        )
    return result.stdout


def collect_git_status(project_root: Path) -> list[dict[str, str]]:
    top_level_raw = _run_git(project_root, ["rev-parse", "--show-toplevel"])
    try:
        top_level_text = top_level_raw.decode("utf-8").strip()
        top_level = Path(top_level_text).resolve(strict=True)
    except (UnicodeDecodeError, OSError, RuntimeError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execute_worktree_git_root_invalid",
            "The Git worktree root could not be resolved.",
        ) from error
    if top_level != project_root:
        raise WorkError(
            ExitCode.CONTRACT,
            "execute_worktree_project_root_mismatch",
            "The project root must be the Git worktree root.",
            {"git_root": str(top_level), "project_root": str(project_root)},
        )
    raw = _run_git(
        project_root,
        [
            "-c",
            "core.quotepath=false",
            "-c",
            "status.relativePaths=false",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )
    return parse_porcelain_v1_z(raw)


def _task_paths(task: dict[str, Any]) -> set[str]:
    return {
        item[field]
        for item in task.get("files", [])
        for field in ("path", "source", "destination")
        if field in item
    }


def _within_directory(path: str, directory: str) -> bool:
    return path == directory or path.startswith(f"{directory}/")


def classify_changes(
    records: list[dict[str, str]],
    *,
    execution_dir: str,
    target_task_id: str,
    target_paths: set[str],
    dependency_paths: dict[str, set[str]],
) -> tuple[list[dict[str, object]], int]:
    changes: list[dict[str, object]] = []
    excluded_count = 0
    for record in records:
        record_paths = [record["path"]]
        if "original_path" in record:
            record_paths.append(record["original_path"])
        if all(_within_directory(path, execution_dir) for path in record_paths):
            excluded_count += 1
            continue

        matching_dependencies = [
            task_id
            for task_id, paths in dependency_paths.items()
            if any(path in paths for path in record_paths)
        ]
        if any(path in target_paths for path in record_paths):
            classification = "target_task"
            matched_task_ids = [target_task_id]
        elif matching_dependencies:
            classification = "completed_dependency"
            matched_task_ids = matching_dependencies
        else:
            classification = "unrelated"
            matched_task_ids = []

        change: dict[str, object] = {
            "index_status": record["index_status"],
            "worktree_status": record["worktree_status"],
            "path": record["path"],
        }
        if "original_path" in record:
            change["original_path"] = record["original_path"]
        change["path_classification"] = classification
        if matched_task_ids:
            change["matched_task_ids"] = matched_task_ids
        changes.append(change)
    return changes, excluded_count


def worktree_snapshot_sha256(
    records: list[dict[str, str]], *, execution_dir: str
) -> str:
    included: list[dict[str, str]] = []
    for record in records:
        record_paths = [record["path"]]
        if "original_path" in record:
            record_paths.append(record["original_path"])
        if all(_within_directory(path, execution_dir) for path in record_paths):
            continue
        included.append(record)
    payload = json.dumps(
        {
            "schema": "work-execute-worktree-snapshot/v1",
            "records": included,
        },
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return canonical_sha256(payload, source="Git worktree snapshot")


def inspect_execute_worktree(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    confirmed_inputs: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    preflight = execute_preflight(
        project_root=project_root,
        user_config_root=user_config_root,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        task_id=task_id,
        confirmed_inputs=confirmed_inputs,
        skill_roots=skill_roots,
    )
    normalized_task = str(preflight["task_path"])
    _, task_path = resolve_project_relative_path(
        project_root, normalized_task, field="task_path"
    )
    task_raw = read_raw(task_path)
    if canonical_sha256(task_raw, source=str(task_path)) != preflight["task_sha256"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_worktree_task_changed",
            "The formal TASK changed after preflight.",
        )
    _, task_contract = parse_markdown_json_contract(task_raw, source=str(task_path))
    tasks = {item["id"]: item for item in task_contract["tasks"]}
    dependencies = list(preflight["dependencies"])
    records = collect_git_status(project_root)
    snapshot_sha256 = worktree_snapshot_sha256(
        records, execution_dir=str(preflight["execution_dir"])
    )
    changes, excluded_count = classify_changes(
        records,
        execution_dir=str(preflight["execution_dir"]),
        target_task_id=task_id,
        target_paths=_task_paths(tasks[task_id]),
        dependency_paths={
            dependency: _task_paths(tasks[dependency]) for dependency in dependencies
        },
    )

    counts = {
        "staged": sum(
            item["index_status"] not in {" ", "?"} for item in changes
        ),
        "unstaged": sum(
            item["worktree_status"] not in {" ", "?"} for item in changes
        ),
        "untracked": sum(
            item["index_status"] == "?" and item["worktree_status"] == "?"
            for item in changes
        ),
        "target_task": sum(
            item["path_classification"] == "target_task" for item in changes
        ),
        "completed_dependency": sum(
            item["path_classification"] == "completed_dependency"
            for item in changes
        ),
        "unrelated": sum(
            item["path_classification"] == "unrelated" for item in changes
        ),
    }
    return {
        "schema": "work-execute-worktree/v1",
        "requirement_id": preflight["requirement_id"],
        "task_spec_id": preflight["task_spec_id"],
        "task_id": task_id,
        "task_sha256": preflight["task_sha256"],
        "task_instructions_sha256": preflight["task_instructions_sha256"],
        "execute_instructions_sha256": preflight["execute_instructions_sha256"],
        "task_status": preflight["task_status"],
        "task_path": preflight["task_path"],
        "index_sha256": preflight["index_sha256"],
        "execution_dir": preflight["execution_dir"],
        "snapshot_sha256": snapshot_sha256,
        "review_status": "required" if changes else "clean",
        "excluded_execution_change_count": excluded_count,
        "counts": counts,
        "changes": changes,
    }
