from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ..contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from ..contracts.task import (
    prepare_task_json_contract,
    validate_task_file,
)
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot


def _task_create_inputs(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    raw_task_path: str,
    raw_execution_dir: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[
    dict[str, Any],
    dict[str, object],
    bytes,
    dict[str, Any],
    bytes,
    str,
    Path,
    str,
    Path,
]:
    contract = parse_json_contract(raw, source=source)
    validation, rendered_task = prepare_task_json_contract(
        raw,
        source=source,
        actual_task_path=raw_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    artifacts = contract["artifacts"]
    normalized_plan, _ = resolve_project_relative_path(
        project_root, raw_plan_path, field="plan_path"
    )
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if (
        normalized_plan != artifacts["plan"]
        or normalized_task != artifacts["task"]
        or normalized_execution != artifacts["execution"]
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "task_create_path_mismatch",
            "The explicit create paths must match the TASK artifact paths.",
        )
    initial_index = build_initial_execution_index(contract, validation)
    rendered_index = render_execution_index(initial_index)
    validate_execution_index(
        rendered_index,
        source="generated execution index",
        expected=initial_index,
    )
    return (
        contract,
        validation,
        rendered_task,
        initial_index,
        rendered_index,
        normalized_task,
        task_path,
        normalized_execution,
        execution_path,
    )


def _write_exclusive(path: Path, content: bytes, *, code: str, label: str) -> None:
    try:
        with path.open("xb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            code,
            f"The {label} target already exists.",
            {"path": str(path)},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            f"{code}_write_failed",
            f"The {label} could not be created.",
            {"path": str(path)},
        ) from error


def _validate_created_pair(
    *,
    project_root: Path,
    user_config_root: str,
    normalized_task: str,
    task_validation: dict[str, object],
    index_path: Path,
    initial_index: dict[str, Any],
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[dict[str, object], dict[str, object]]:
    stored_task = validate_task_file(
        project_root,
        user_config_root,
        normalized_task,
        skill_roots=skill_roots,
    )
    if stored_task != task_validation:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_post_write_mismatch",
            "The stored TASK does not match the validated canonical TASK.",
        )
    stored_index = validate_execution_index(
        read_raw(index_path),
        source=str(index_path),
        expected=initial_index,
    )
    return stored_task, stored_index


def create_task_artifacts(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    raw_task_path: str,
    raw_execution_dir: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    (
        _,
        task_validation,
        rendered_task,
        initial_index,
        rendered_index,
        normalized_task,
        task_path,
        normalized_execution,
        execution_path,
    ) = _task_create_inputs(
        raw,
        source=source,
        raw_plan_path=raw_plan_path,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if task_path.exists() or execution_path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "task_create_target_exists",
            "TASK create requires both the TASK and execution directory to be absent.",
            {
                "task_exists": task_path.exists(),
                "execution_exists": execution_path.exists(),
            },
        )
    try:
        task_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "task_create_parent_failed",
            "A TASK create parent directory could not be created.",
        ) from error
    _write_exclusive(
        task_path,
        rendered_task,
        code="task_already_exists",
        label="TASK",
    )
    try:
        execution_path.mkdir()
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execution_directory_already_exists",
            "The execution directory appeared after the TASK was created.",
            {"path": normalized_execution},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execution_directory_create_failed",
            "The execution directory could not be created after the TASK was created.",
            {"path": normalized_execution},
        ) from error
    index_path = execution_path / "index.md"
    _write_exclusive(
        index_path,
        rendered_index,
        code="execution_index_already_exists",
        label="execution index",
    )
    stored_task, stored_index = _validate_created_pair(
        project_root=project_root,
        user_config_root=user_config_root,
        normalized_task=normalized_task,
        task_validation=task_validation,
        index_path=index_path,
        initial_index=initial_index,
        skill_roots=skill_roots,
    )
    return {
        "schema": "work-task-create/v1",
        "requirement_id": stored_task["requirement_id"],
        "spec_id": stored_task["spec_id"],
        "task_path": normalized_task,
        "execution_dir": normalized_execution,
        "task_sha256": stored_task["task_sha256"],
        "index_sha256": stored_index["index_sha256"],
        "status": "created",
    }


def recover_task_create(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    raw_task_path: str,
    raw_execution_dir: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    (
        _,
        task_validation,
        rendered_task,
        initial_index,
        rendered_index,
        normalized_task,
        task_path,
        normalized_execution,
        execution_path,
    ) = _task_create_inputs(
        raw,
        source=source,
        raw_plan_path=raw_plan_path,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if not task_path.is_file() or read_raw(task_path) != rendered_task:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "Recovery requires the same canonical TASK to already exist.",
            {"task_path": normalized_task},
        )
    if execution_path.exists() and not execution_path.is_dir():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "The execution target exists but is not a directory.",
            {"execution_dir": normalized_execution},
        )
    if not execution_path.exists():
        try:
            execution_path.parent.mkdir(parents=True, exist_ok=True)
            execution_path.mkdir()
        except OSError as error:
            raise WorkError(
                ExitCode.IO_FAILURE,
                "execution_directory_create_failed",
                "The missing execution directory could not be created during recovery.",
                {"execution_dir": normalized_execution},
            ) from error
    entries = list(execution_path.iterdir())
    index_path = execution_path / "index.md"
    if not entries:
        _write_exclusive(
            index_path,
            rendered_index,
            code="execution_index_already_exists",
            label="execution index",
        )
        recovered = True
        status = "recovered"
    elif len(entries) == 1 and entries[0] == index_path and index_path.is_file():
        if read_raw(index_path) != rendered_index:
            raise WorkError(
                ExitCode.WORKFLOW_STATE,
                "unrecoverable_task_create_state",
                "The existing execution index is not the expected initial index.",
                {"execution_dir": normalized_execution},
            )
        recovered = False
        status = "already_completed"
    else:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "The execution directory contains unknown or non-initial content.",
            {"execution_dir": normalized_execution},
        )
    stored_task, stored_index = _validate_created_pair(
        project_root=project_root,
        user_config_root=user_config_root,
        normalized_task=normalized_task,
        task_validation=task_validation,
        index_path=index_path,
        initial_index=initial_index,
        skill_roots=skill_roots,
    )
    return {
        "schema": "work-task-create-recovery/v1",
        "requirement_id": stored_task["requirement_id"],
        "spec_id": stored_task["spec_id"],
        "task_path": normalized_task,
        "execution_dir": normalized_execution,
        "task_sha256": stored_task["task_sha256"],
        "index_sha256": stored_index["index_sha256"],
        "recovered": recovered,
        "status": status,
    }
