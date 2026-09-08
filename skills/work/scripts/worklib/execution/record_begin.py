from __future__ import annotations

import copy
import os
from pathlib import Path
from typing import Any

from ..contracts.attempt import validate_attempt_file
from ..foundation.errors import ExitCode, WorkError
from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot
from ..contracts.task import validate_task_contract
from .context import read_contract, find_task_row, validate_execution_identity
from .instructions import validate_execute_instructions
from .records import BASE_RECORD_PATTERN, next_record_id, formal_record_kind



def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def _write_lock_update(
    *,
    index_path: Path,
    index_raw: bytes,
    target_index: dict[str, Any],
    temporary_path: Path,
) -> None:
    rendered = render_execution_index(target_index)
    validate_execution_index(rendered, source="generated record-begin index")
    try:
        with temporary_path.open("xb") as output:
            output.write(rendered)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "record_begin_transaction_present",
            "A record-begin transaction already requires recovery.",
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": "lock_update_prepared",
            },
        ) from error
    except OSError as error:
        details: dict[str, object] = {"path": str(temporary_path)}
        if temporary_path.exists():
            details.update(
                {
                    "recovery_required": True,
                    "transaction_stage": "lock_update_partial",
                }
            )
        raise WorkError(
            ExitCode.IO_FAILURE,
            "record_begin_prepare_failed",
            "The record-begin index update could not be prepared.",
            details,
        ) from error
    if read_raw(index_path) != index_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_index_changed",
            "The execution index changed during record begin.",
            path=str(temporary_path),
            recovery_required=True,
            transaction_stage="lock_update_prepared",
        )
    try:
        os.replace(temporary_path, index_path)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "record_begin_replace_failed",
            "The prepared record-begin index could not be installed.",
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": "lock_update_prepared",
            },
        ) from error
    stored = read_raw(index_path)
    validate_execution_index(stored, source=str(index_path))
    if stored != rendered:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_write_mismatch",
            "The stored execution index does not match the prepared bytes.",
        )


def begin_record(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    base_record_id: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    if not BASE_RECORD_PATTERN.fullmatch(base_record_id):
        _error(
            ExitCode.CONTRACT,
            "record_begin_invalid_base_record_id",
            "record_id must be a base CMD-, OP-, or VAL- identifier.",
            record_id=base_record_id,
        )
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if not execution_path.is_dir():
        _error(
            ExitCode.IO_FAILURE,
            "record_begin_execution_directory_missing",
            "The execution directory does not exist.",
            path=normalized_execution,
        )
    transaction_files = sorted(
        path.name
        for path in execution_path.glob(".work-record-begin-*.tmp")
        if path.is_file()
    )
    if transaction_files:
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_begin_transaction_present",
            "A record-begin transaction already requires recovery.",
            files=transaction_files,
            recovery_required=True,
        )
    task_raw = read_raw(task_path)
    task_validation = validate_task_contract(
        task_raw,
        source=str(task_path),
        actual_task_path=normalized_task,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=False,
        skill_roots=skill_roots,
    )
    task_contract = parse_json_contract(task_raw, source=str(task_path))
    if (
        task_contract["artifacts"]["task"] != normalized_task
        or task_contract["artifacts"]["execution"] != normalized_execution
    ):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
        )
    try:
        task = next(item for item in task_contract["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "record_begin_task_not_found",
            "The requested TASK is not present in the formal TASK.",
            {"task_id": task_id},
        ) from error
    record_kind = formal_record_kind(task, base_record_id)

    index_relative = f"{normalized_execution}/index.json"
    _, index_path = resolve_project_relative_path(
        project_root, index_relative, field="execution_index"
    )
    index_raw, index = read_contract(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    row = find_task_row(index, task_id)
    if row["status"] != "in_progress" or "latest_attempt" not in row:
        _error(
            ExitCode.WORKFLOW_STATE,
            "record_begin_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            status=row["status"],
        )
    attempt_id = row["latest_attempt"]
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}.json"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    _, attempt = read_contract(attempt_path)
    if attempt["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "record_begin_attempt_not_in_progress",
            "The latest Attempt is not in progress.",
        )
    row = validate_execution_identity(
        task_contract=task_contract,
        task_validation=task_validation,
        index=index,
        attempt=attempt,
        task_id=task_id,
    )

    lock = index.get("lock")
    expected_lock = {
        "kind": "execution",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "execute_instructions_sha256": attempt["execute_instructions_sha256"],
    }
    if not isinstance(lock, dict) or any(
        lock.get(field) != value for field, value in expected_lock.items()
    ):
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_begin_lock_mismatch",
            "The execution lock does not match the active Attempt.",
            expected=expected_lock,
            actual=lock,
        )
    if "record_id" in lock:
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_begin_record_already_reserved",
            "The execution lock already reserves a record.",
            record_id=lock["record_id"],
        )

    validate_execute_instructions(task, attempt, operation="record_begin")

    record_id = next_record_id(base_record_id, attempt)
    updated_index = copy.deepcopy(index)
    updated_index["lock"]["record_id"] = record_id
    safe_record = record_id.replace("#", "-retry-")
    temporary_path = execution_path / (
        f".work-record-begin-{task_id}-{attempt_id}-{safe_record}.tmp"
    )
    _write_lock_update(
        index_path=index_path,
        index_raw=index_raw,
        target_index=updated_index,
        temporary_path=temporary_path,
    )
    return {
        "schema": "work-record-begin/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "base_record_id": base_record_id,
        "record_id": record_id,
        "record_kind": record_kind,
        "index_path": index_relative,
        "lock_status": "record_reserved",
    }
