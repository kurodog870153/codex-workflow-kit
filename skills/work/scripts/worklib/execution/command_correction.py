from __future__ import annotations

import copy
import os
from pathlib import Path
from typing import Any

from ..contracts.attempt import (
    canonicalize_command_correction,
    validate_attempt_file,
)
from ..foundation.errors import ExitCode, WorkError
from .record_begin import (
    _formal_record,
    _read_contract,
    _task_row,
    _validate_execute_instructions,
    _validate_identity,
    next_record_id,
)
from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract, parse_markdown_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot
from ..contracts.task import validate_task_contract


REQUEST_SCHEMA = "work-command-correction-request/v1"


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_command_correction_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "command_correction_expected_object",
            "A JSON object is required.",
        )
    required = {
        "schema",
        "record_id",
        "original_command",
        "actual_command",
        "reason",
        "authorization_evidence",
    }
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "command_correction_invalid_fields",
            "The command-correction request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "command_correction_invalid_schema",
            "The command-correction request schema is invalid.",
        )
    record_id = request["record_id"]
    if not isinstance(record_id, str) or not record_id.startswith("CMD-"):
        _error(
            ExitCode.CONTRACT,
            "command_correction_invalid_record_id",
            "record_id must identify a reserved CMD record.",
            record_id=record_id,
        )
    correction = canonicalize_command_correction(
        {field: request[field] for field in required - {"schema", "record_id"}}
    )
    return {
        "schema": REQUEST_SCHEMA,
        "record_id": record_id,
        "correction": correction,
    }


def _formal_command(task: dict[str, Any], base_record_id: str) -> dict[str, Any]:
    try:
        command = next(
            item for item in task.get("commands", []) if item["id"] == base_record_id
        )
    except StopIteration as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "command_correction_command_not_found",
            "The reserved command is not defined by the target TASK.",
            {"record_id": base_record_id},
        ) from error
    field = "argv" if command.get("mode") == "argv" else "script"
    return {"mode": command.get("mode"), field: copy.deepcopy(command.get(field))}


def _write_index_update(
    *,
    index_path: Path,
    index_raw: bytes,
    target_index: dict[str, Any],
    temporary_path: Path,
) -> None:
    rendered = render_execution_index(target_index)
    validate_execution_index(rendered, source="generated command-correction index")
    try:
        with temporary_path.open("xb") as output:
            output.write(rendered)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "command_correction_transaction_present",
            "A command-correction transaction already requires recovery.",
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
            "command_correction_prepare_failed",
            "The command-correction index update could not be prepared.",
            details,
        ) from error
    if read_raw(index_path) != index_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "command_correction_index_changed",
            "The execution index changed during command correction.",
            path=str(temporary_path),
            recovery_required=True,
            transaction_stage="lock_update_prepared",
        )
    try:
        os.replace(temporary_path, index_path)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "command_correction_replace_failed",
            "The prepared command-correction index could not be installed.",
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": "lock_update_prepared",
            },
        ) from error
    if read_raw(index_path) != rendered:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "command_correction_stored_bytes_mismatch",
            "The installed command-correction index bytes are not canonical.",
            recovery_required=True,
            transaction_stage="lock_updated",
        )


def record_command_correction(
    raw: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    request = parse_command_correction_request(raw, source=source)
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if not execution_path.is_dir():
        _error(
            ExitCode.IO_FAILURE,
            "command_correction_execution_directory_missing",
            "The execution directory does not exist.",
            path=normalized_execution,
        )
    transaction_files = sorted(
        path.name
        for path in execution_path.glob(".work-command-correction-*.tmp")
        if path.is_file()
    )
    if transaction_files:
        _error(
            ExitCode.LOCK_CONFLICT,
            "command_correction_transaction_present",
            "A command-correction transaction already requires recovery.",
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
    _, task_contract = parse_markdown_json_contract(task_raw, source=str(task_path))
    if (
        task_contract["artifacts"]["task"] != normalized_task
        or task_contract["artifacts"]["execution"] != normalized_execution
    ):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "command_correction_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
        )
    try:
        task = next(item for item in task_contract["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "command_correction_task_not_found",
            "The requested TASK is not present in the formal TASK.",
            {"task_id": task_id},
        ) from error

    index_relative = f"{normalized_execution}/index.md"
    _, index_path = resolve_project_relative_path(
        project_root, index_relative, field="execution_index"
    )
    index_raw, index = _read_contract(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    row = _task_row(index, task_id)
    if row["status"] != "in_progress" or "latest_attempt" not in row:
        _error(
            ExitCode.WORKFLOW_STATE,
            "command_correction_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            status=row["status"],
        )
    attempt_id = row["latest_attempt"]
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}.md"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    _, attempt = _read_contract(attempt_path)
    if attempt["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "command_correction_attempt_not_in_progress",
            "The latest Attempt is not in progress.",
        )
    _validate_identity(
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
        "record_id": request["record_id"],
    }
    if not isinstance(lock, dict) or any(
        lock.get(field) != value for field, value in expected_lock.items()
    ):
        _error(
            ExitCode.LOCK_CONFLICT,
            "command_correction_lock_mismatch",
            "The execution lock does not match the reserved command.",
            expected=expected_lock,
            actual=lock,
        )
    if "command_correction" in lock:
        _error(
            ExitCode.LOCK_CONFLICT,
            "command_correction_already_recorded",
            "The reserved command already has a correction.",
        )
    record_id = request["record_id"]
    base_record_id = record_id.split("#", 1)[0]
    if _formal_record(task, base_record_id) != "command":
        _error(
            ExitCode.CONTRACT,
            "command_correction_non_command_record",
            "Only a formal CMD record can have a command correction.",
        )
    expected_record_id = next_record_id(base_record_id, attempt)
    if record_id != expected_record_id:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "command_correction_retry_sequence_mismatch",
            "The reserved command ID is not the next record instance.",
            expected=expected_record_id,
            actual=record_id,
        )
    formal_command = canonicalize_command_correction(
        {
            "original_command": _formal_command(task, base_record_id),
            "actual_command": request["correction"]["actual_command"],
            "reason": request["correction"]["reason"],
            "authorization_evidence": request["correction"]["authorization_evidence"],
        },
        location="formal_command_correction",
    )
    if formal_command["original_command"] != request["correction"]["original_command"]:
        _error(
            ExitCode.CONTRACT,
            "command_correction_original_mismatch",
            "original_command does not match the formal TASK command.",
            expected=formal_command["original_command"],
            actual=request["correction"]["original_command"],
        )

    _validate_execute_instructions(
        task,
        attempt,
        operation="command_correction",
    )

    updated_index = copy.deepcopy(index)
    updated_index["lock"]["command_correction"] = request["correction"]
    safe_record = record_id.replace("#", "-retry-")
    temporary_path = execution_path / (
        f".work-command-correction-{task_id}-{attempt_id}-{safe_record}.tmp"
    )
    _write_index_update(
        index_path=index_path,
        index_raw=index_raw,
        target_index=updated_index,
        temporary_path=temporary_path,
    )
    validate_execution_index(read_raw(index_path), source=str(index_path))
    return {
        "schema": "work-command-correction/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "record_id": record_id,
        "index_path": index_relative,
        "correction_status": "recorded",
        "lock_status": "record_reserved",
    }
