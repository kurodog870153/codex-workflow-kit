from __future__ import annotations

from pathlib import Path
from typing import Any

from ...models.execution.inspection import (
    ExecuteWorktreeContract,
)
from ...models.common.errors import ExitCode, WorkError
from .preflight import execute_preflight
from ...services.execution.worktree import (
    canonical_sha256, parse_json_contract,
    raw_sha256, read_raw, resolve_project_relative_path,
    run_read_only_git as _run_git,
)
from ...services.skill_catalog import SkillRoot
from ...services.worktree.classification import classify_changes
from ...services.worktree.fingerprint import worktree_snapshot_sha256
from ...services.worktree.parsing import parse_porcelain_v1_z


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


def inspect_execute_worktree(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    confirmed_inputs: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None,
    _context_out: dict[str, object] | None = None,
    _prevalidated_context: dict[str, object] | None = None,
    operations=None,
) -> dict[str, object]:
    preflight_context: dict[str, object] = {}
    preflight = execute_preflight(
        project_root=project_root,
        user_config_root=user_config_root,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        task_id=task_id,
        confirmed_inputs=confirmed_inputs,
        skill_roots=skill_roots,
        _context_out=preflight_context,
        _prevalidated_context=_prevalidated_context,
        operations=operations,
    )
    normalized_task = str(preflight["task_path"])
    _, task_path = resolve_project_relative_path(
        project_root, normalized_task, field="task_path"
    )
    item_path = task_path.parent / "tasks" / f"{task_id}.json"
    unchanged = (
        raw_sha256(read_raw(task_path)) == preflight.get("task_index_sha256")
        and raw_sha256(read_raw(item_path)) == preflight.get("task_item_sha256")
    )
    if not unchanged:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_worktree_task_changed",
            "The formal TASK changed after preflight.",
        )
    task_context = preflight_context.get("context")
    if task_context is None:
        task_context = operations.load_task_execution_context(
            project_root, user_config_root, normalized_task, task_id,
            skill_roots=skill_roots,
        )
    else:
        task_context = operations.recheck_task_execution_context(
            project_root, user_config_root, normalized_task, task_context,
            skill_roots=skill_roots,
        )
    validation = task_context["validation"]
    assert isinstance(validation, dict)
    fingerprints = {
        "task_collection_sha256": validation["task_collection_sha256"],
        "task_index_sha256": validation["task_index_sha256"],
        "task_item_sha256": validation["task_item_sha256"][task_id],
    }
    if any(preflight.get(key) != value for key, value in fingerprints.items()):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_worktree_task_changed",
            "The formal TASK changed after preflight.",
        )
    if _context_out is not None:
        _context_out["context"] = task_context
    task_contract = task_context["contract"]
    assert isinstance(task_contract, dict)
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
    return ExecuteWorktreeContract.model_validate({
        "schema": "work-execute-worktree/v1",
        "requirement_id": preflight["requirement_id"],
        "task_spec_id": preflight["task_spec_id"],
        "task_id": task_id,
        **{key: preflight[key] for key in fingerprints},
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
    }).to_canonical_dict()
