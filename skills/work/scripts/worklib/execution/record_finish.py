from __future__ import annotations

import copy
import os
from pathlib import Path
from typing import Any

from ..contracts.attempt import (
    canonicalize_attempt_contract,
    render_attempt_contract,
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


REQUEST_SCHEMA = "work-record-finish-request/v1"


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_record_finish_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "record_finish_expected_object",
            "A JSON object is required.",
        )
    required = {"schema", "record"}
    optional = {"modified_files"}
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required - optional)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "record_finish_invalid_fields",
            "The record-finish request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "record_finish_invalid_schema",
            "The record-finish request schema is invalid.",
        )
    if not isinstance(request["record"], dict):
        _error(
            ExitCode.CONTRACT,
            "record_finish_invalid_record",
            "record must be a JSON object.",
        )
    if "modified_files" in request:
        files = request["modified_files"]
        if not isinstance(files, list) or not files:
            _error(
                ExitCode.CONTRACT,
                "record_finish_invalid_modified_files",
                "modified_files must be a non-empty array when present.",
            )
        if any(not isinstance(item, str) or not item for item in files):
            _error(
                ExitCode.CONTRACT,
                "record_finish_invalid_modified_file",
                "Every modified file must be a non-empty string.",
            )
    return request


def _overall_result(records: list[dict[str, Any]]) -> dict[str, Any] | None:
    operations = [item for item in records if item["kind"] == "operation"]
    if not operations:
        return None
    effective = [item["id"] for item in operations if item["outcome"] == "success"]
    not_effective = [
        item["id"] for item in operations if item["outcome"] == "failure"
    ]
    unknown = [item["id"] for item in operations if item["outcome"] == "unknown"]
    if unknown:
        status = "uncertain_result"
    elif effective and not_effective:
        status = "partial_success"
    elif not_effective:
        status = "failure"
    else:
        status = "complete_success"
    result: dict[str, Any] = {"status": status}
    for field, values in (
        ("effective", effective),
        ("not_effective", not_effective),
        ("unknown", unknown),
    ):
        if values:
            result[field] = values
    return result


def build_finished_attempt(
    attempt: dict[str, Any],
    request: dict[str, Any],
    *,
    project_root: Path,
    expected_record_id: str,
    expected_kind: str,
    command_correction: dict[str, Any] | None = None,
) -> dict[str, Any]:
    record = copy.deepcopy(request["record"])
    if record.get("id") != expected_record_id:
        _error(
            ExitCode.CONTRACT,
            "record_finish_record_id_mismatch",
            "The result record ID does not match the execution lock.",
            expected=expected_record_id,
            actual=record.get("id"),
        )
    if record.get("kind") != expected_kind:
        _error(
            ExitCode.CONTRACT,
            "record_finish_record_kind_mismatch",
            "The result record kind does not match the formal TASK record.",
            expected=expected_kind,
            actual=record.get("kind"),
        )
    if "correction" in record:
        _error(
            ExitCode.CONTRACT,
            "record_finish_untrusted_command_correction",
            "A record-finish request cannot supply command correction data.",
        )
    if command_correction is not None:
        if expected_kind != "command":
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "record_finish_invalid_command_correction_lock",
                "Only a reserved command can carry command correction data.",
            )
        record["correction"] = copy.deepcopy(command_correction)
    candidate = copy.deepcopy(attempt)
    candidate["records"].append(record)
    if "modified_files" in request:
        current_files = list(candidate.get("modified_files", []))
        for path in request["modified_files"]:
            if path not in current_files:
                current_files.append(path)
        candidate["modified_files"] = current_files
    overall = _overall_result(candidate["records"])
    if overall is None:
        candidate.pop("overall_result", None)
    else:
        candidate["overall_result"] = overall
    return canonicalize_attempt_contract(candidate, project_root=project_root)


def _prepare_and_replace(
    *,
    source_path: Path,
    source_bytes: bytes,
    target_bytes: bytes,
    temporary_path: Path,
    stage: str,
) -> None:
    try:
        with temporary_path.open("xb") as output:
            output.write(target_bytes)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "record_finish_transaction_present",
            "A record-finish transaction already requires recovery.",
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    except OSError as error:
        details: dict[str, object] = {"path": str(temporary_path)}
        if temporary_path.exists():
            details.update(
                {"recovery_required": True, "transaction_stage": stage}
            )
        raise WorkError(
            ExitCode.IO_FAILURE,
            "record_finish_prepare_failed",
            "The record-finish update could not be prepared.",
            details,
        ) from error
    if read_raw(source_path) != source_bytes:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_finish_source_changed",
            "A transaction source changed before replacement.",
            path=str(temporary_path),
            recovery_required=True,
            transaction_stage=stage,
        )
    try:
        os.replace(temporary_path, source_path)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "record_finish_replace_failed",
            "The prepared record-finish update could not be installed.",
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    if read_raw(source_path) != target_bytes:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_finish_write_mismatch",
            "The installed record-finish bytes do not match the prepared bytes.",
            recovery_required=True,
            transaction_stage=stage,
        )


def _transaction_error(
    error: WorkError,
    *,
    attempt_path: Path,
    original_attempt: bytes,
    attempt_temporary: Path,
    index_temporary: Path,
) -> WorkError:
    if error.details.get("recovery_required"):
        return error
    if index_temporary.exists():
        stage = "lock_update_prepared"
    elif read_raw(attempt_path) != original_attempt:
        stage = "attempt_updated"
    elif attempt_temporary.exists():
        stage = "attempt_update_prepared"
    else:
        return error
    details = dict(error.details)
    details.update({"recovery_required": True, "transaction_stage": stage})
    return WorkError(error.exit_code, error.code, error.message, details)


def finish_record(
    raw_request: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    request = parse_record_finish_request(raw_request, source=source)
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if not execution_path.is_dir():
        _error(
            ExitCode.IO_FAILURE,
            "record_finish_execution_directory_missing",
            "The execution directory does not exist.",
            path=normalized_execution,
        )
    transaction_files = sorted(
        path.name
        for path in execution_path.glob(".work-record-finish-*.tmp")
        if path.is_file()
    )
    if transaction_files:
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_finish_transaction_present",
            "A record-finish transaction already requires recovery.",
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
            "record_finish_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
        )
    try:
        task = next(item for item in task_contract["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "record_finish_task_not_found",
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
            "record_finish_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            status=row["status"],
        )
    attempt_id = row["latest_attempt"]
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}.md"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    attempt_raw, attempt = _read_contract(attempt_path)
    if attempt["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "record_finish_attempt_not_in_progress",
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
    if not isinstance(lock, dict) or lock.get("kind") != "execution":
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_finish_execution_lock_required",
            "A matching execution lock is required.",
            lock=lock,
        )
    expected_lock = {
        "task_id": task_id,
        "attempt_id": attempt_id,
        "execute_instructions_sha256": attempt["execute_instructions_sha256"],
    }
    if any(lock.get(field) != value for field, value in expected_lock.items()):
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_finish_lock_mismatch",
            "The execution lock does not match the active Attempt.",
            expected=expected_lock,
            actual=lock,
        )
    record_id = lock.get("record_id")
    if not isinstance(record_id, str):
        _error(
            ExitCode.LOCK_CONFLICT,
            "record_finish_record_not_reserved",
            "The execution lock does not reserve a record.",
        )
    base_record_id = record_id.split("#", 1)[0]
    record_kind = _formal_record(task, base_record_id)
    expected_record_id = next_record_id(base_record_id, attempt)
    if record_id != expected_record_id:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_finish_retry_sequence_mismatch",
            "The reserved record ID is not the next record instance.",
            expected=expected_record_id,
            actual=record_id,
        )

    _validate_execute_instructions(task, attempt, operation="record_finish")

    finished_attempt = build_finished_attempt(
        attempt,
        request,
        project_root=project_root,
        expected_record_id=record_id,
        expected_kind=record_kind,
        command_correction=lock.get("command_correction"),
    )
    rendered_attempt = render_attempt_contract(
        finished_attempt, project_root=project_root
    )
    updated_index = copy.deepcopy(index)
    updated_index["lock"].pop("record_id")
    updated_index["lock"].pop("command_correction", None)
    rendered_index = render_execution_index(updated_index)
    validate_execution_index(rendered_index, source="generated record-finish index")

    safe_record = record_id.replace("#", "-retry-")
    attempt_temporary = execution_path / (
        f".work-record-finish-{task_id}-{attempt_id}-{safe_record}-attempt.tmp"
    )
    index_temporary = execution_path / (
        f".work-record-finish-{task_id}-{attempt_id}-{safe_record}-index.tmp"
    )
    try:
        _prepare_and_replace(
            source_path=attempt_path,
            source_bytes=attempt_raw,
            target_bytes=rendered_attempt,
            temporary_path=attempt_temporary,
            stage="attempt_update_prepared",
        )
        validate_attempt_file(project_root, attempt_relative)
        _prepare_and_replace(
            source_path=index_path,
            source_bytes=index_raw,
            target_bytes=rendered_index,
            temporary_path=index_temporary,
            stage="lock_update_prepared",
        )
        validate_execution_index(read_raw(index_path), source=str(index_path))
    except WorkError as error:
        transaction_error = _transaction_error(
            error,
            attempt_path=attempt_path,
            original_attempt=attempt_raw,
            attempt_temporary=attempt_temporary,
            index_temporary=index_temporary,
        )
        if transaction_error is error:
            raise
        raise transaction_error from error
    return {
        "schema": "work-record-finish/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "record_id": record_id,
        "record_kind": record_kind,
        "attempt_path": attempt_relative,
        "index_path": index_relative,
        "record_status": "recorded",
        "lock_status": "attempt_held",
    }
