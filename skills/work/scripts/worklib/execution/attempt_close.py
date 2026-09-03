from __future__ import annotations

import copy
import os
from datetime import datetime
from pathlib import Path
from typing import Any

from ..contracts.attempt import (
    canonicalize_attempt_contract,
    render_attempt_contract,
    validate_attempt_file,
)
from ..foundation.errors import ExitCode, WorkError
from .record_begin import (
    BASE_EXECUTE_REFERENCES,
    RECOVERY_REFERENCE,
    _read_contract,
    _task_row,
    _validate_identity,
)
from ..contracts.execution_index import (
    derive_overall_status,
    render_execution_index,
    validate_execution_index,
)
from ..foundation.fingerprint import read_raw
from ..instructions.selection import build_instruction_selection
from ..foundation.markdown import parse_json_contract, parse_markdown_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot
from ..contracts.task import validate_task_contract


REQUEST_SCHEMA = "work-attempt-close-request/v1"
BLOCKING_STOPPED_TYPES = {
    "specification_defect",
    "instructions_changed",
    "external_operation_failed",
}


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_attempt_close_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "attempt_close_expected_object",
            "A JSON object is required.",
        )
    required = {"schema", "status"}
    optional = {"final_type", "reason"}
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required - optional)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_invalid_fields",
            "The attempt-close request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_invalid_schema",
            "The attempt-close request schema is invalid.",
        )
    status = request["status"]
    if status not in {"completed", "stopped", "blocked"}:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_invalid_status",
            "status must be completed, stopped, or blocked.",
            status=status,
        )
    final_fields = {"final_type", "reason"} & set(request)
    if status == "completed" and final_fields:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_unexpected_final_details",
            "A completed Attempt cannot include final_type or reason.",
            fields=sorted(final_fields),
        )
    if status != "completed":
        missing_final = {"final_type", "reason"} - set(request)
        if missing_final:
            _error(
                ExitCode.CONTRACT,
                "attempt_close_missing_final_details",
                "A stopped or blocked Attempt requires final_type and reason.",
                missing=sorted(missing_final),
            )
        if any(
            not isinstance(request[field], str) or not request[field].strip()
            for field in ("final_type", "reason")
        ):
            _error(
                ExitCode.CONTRACT,
                "attempt_close_empty_final_detail",
                "final_type and reason must be non-empty strings.",
            )
    return request


def _timestamp(now: datetime | None) -> str:
    value = now or datetime.now().astimezone()
    if value.tzinfo is None or value.utcoffset() is None:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_naive_time",
            "Attempt end time must include a timezone offset.",
        )
    return value.replace(second=0, microsecond=0).isoformat(timespec="minutes")


def _latest_record_outcomes(attempt: dict[str, Any]) -> dict[str, str]:
    outcomes: dict[str, str] = {}
    for carried in attempt.get("carried_records", []):
        base_id = carried["record_id"].split("#", 1)[0]
        if base_id.startswith("VAL-"):
            outcomes[base_id] = "passed"
    for record in attempt["records"]:
        base_id = record["id"].split("#", 1)[0]
        if record["kind"] == "validation":
            outcomes[base_id] = record["outcome"]
    return outcomes


def _validate_completed_coverage(
    *, task: dict[str, Any], attempt: dict[str, Any]
) -> None:
    required = [item["id"] for item in task.get("validations", [])]
    outcomes = _latest_record_outcomes(attempt)
    missing = [record_id for record_id in required if record_id not in outcomes]
    failed = [
        record_id
        for record_id in required
        if outcomes.get(record_id) not in {None, "passed"}
    ]
    if missing or failed:
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_close_incomplete_validations",
            "A completed Attempt requires every formal validation to pass.",
            missing=missing,
            failed=failed,
        )


def build_closed_attempt(
    attempt: dict[str, Any],
    request: dict[str, Any],
    *,
    project_root: Path,
    ended_at: str,
) -> dict[str, Any]:
    candidate = copy.deepcopy(attempt)
    candidate["status"] = request["status"]
    if request["status"] == "completed":
        candidate.pop("final_type", None)
        candidate.pop("reason", None)
    else:
        candidate["final_type"] = request["final_type"]
        candidate["reason"] = request["reason"]
    candidate["ended_at"] = ended_at
    return canonicalize_attempt_contract(candidate, project_root=project_root)


def _task_status(request: dict[str, Any]) -> str:
    if request["status"] == "completed":
        return "completed"
    if request["status"] == "blocked":
        return "blocked"
    if request["final_type"] in BLOCKING_STOPPED_TYPES:
        return "blocked"
    return "pending_retry"


def _validate_execute_instruction_close_state(
    task: dict[str, Any],
    attempt: dict[str, Any],
    request: dict[str, Any],
) -> dict[str, object]:
    selection = task["instruction_selection"]
    references = list(BASE_EXECUTE_REFERENCES)
    if "continued_from" in attempt:
        references.append(RECOVERY_REFERENCE)
    current = build_instruction_selection(
        skill_root=Path(__file__).resolve().parents[3],
        mode="execute",
        selected_paths=selection["selected_paths"],
        reference_names=references,
    )
    if (
        current["selected_paths"] != selection["selected_paths"]
        or current["resolved_paths"] != selection["resolved_paths"]
    ):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_close_execute_instruction_hierarchy_mismatch",
            "The current Execute hierarchy does not match the target TASK.",
        )
    instructions_changed = (
        current["instructions_sha256"]
        != attempt["execute_instructions_sha256"]
    )
    closing_for_instruction_change = (
        request["status"] == "stopped"
        and request.get("final_type") == "instructions_changed"
    )
    if instructions_changed != closing_for_instruction_change:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_close_execute_instructions_state_mismatch",
            "The requested close reason does not match the Execute instruction fingerprint state.",
            expected=attempt["execute_instructions_sha256"],
            actual=current["instructions_sha256"],
            closing_for_instruction_change=closing_for_instruction_change,
        )
    return current


def build_closed_index(
    index: dict[str, Any],
    *,
    task_id: str,
    attempt_id: str,
    request: dict[str, Any],
) -> dict[str, Any]:
    task_status = _task_status(request)
    updated_index = copy.deepcopy(index)
    updated_row = _task_row(updated_index, task_id)
    updated_row["status"] = task_status
    if task_status == "completed":
        updated_row.pop("status_reason", None)
    else:
        updated_row["status_reason"] = {"kind": "attempt", "ref": attempt_id}
    updated_index.pop("lock")
    updated_index["overall_status"] = derive_overall_status(
        [item["status"] for item in updated_index["tasks"]]
    )
    return updated_index


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
            "attempt_close_transaction_present",
            "An attempt-close transaction already requires recovery.",
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
            "attempt_close_prepare_failed",
            "The attempt-close update could not be prepared.",
            details,
        ) from error
    if read_raw(source_path) != source_bytes:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_close_source_changed",
            "A source artifact changed during Attempt close.",
            path=str(temporary_path),
            recovery_required=True,
            transaction_stage=stage,
        )
    try:
        os.replace(temporary_path, source_path)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "attempt_close_replace_failed",
            "The prepared attempt-close update could not be installed.",
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    if read_raw(source_path) != target_bytes:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_close_stored_bytes_mismatch",
            "The installed attempt-close bytes do not match the prepared bytes.",
            recovery_required=True,
            transaction_stage=stage.replace("_prepared", "_updated"),
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
        stage = "index_update_prepared"
    elif read_raw(attempt_path) != original_attempt:
        stage = "attempt_updated"
    elif attempt_temporary.exists():
        stage = "attempt_update_prepared"
    else:
        return error
    return WorkError(
        error.exit_code,
        error.code,
        error.message,
        {
            **error.details,
            "recovery_required": True,
            "transaction_stage": stage,
        },
    )


def close_attempt(
    raw: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    now: datetime | None = None,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    request = parse_attempt_close_request(raw, source=source)
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if not execution_path.is_dir():
        _error(
            ExitCode.IO_FAILURE,
            "attempt_close_execution_directory_missing",
            "The execution directory does not exist.",
            path=normalized_execution,
        )
    transaction_files = sorted(
        path.name for path in execution_path.glob(".work-*.tmp") if path.is_file()
    )
    if transaction_files:
        _error(
            ExitCode.LOCK_CONFLICT,
            "attempt_close_transaction_present",
            "An execution transaction already requires recovery.",
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
            "attempt_close_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
        )
    try:
        task = next(item for item in task_contract["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "attempt_close_task_not_found",
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
            "attempt_close_task_not_in_progress",
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
            "attempt_close_attempt_not_in_progress",
            "The latest Attempt is already closed.",
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
    }
    if not isinstance(lock, dict) or any(
        lock.get(field) != value for field, value in expected_lock.items()
    ):
        _error(
            ExitCode.LOCK_CONFLICT,
            "attempt_close_lock_mismatch",
            "The execution lock does not match the active Attempt.",
            expected=expected_lock,
            actual=lock,
        )
    if "record_id" in lock or "command_correction" in lock:
        _error(
            ExitCode.LOCK_CONFLICT,
            "attempt_close_record_reserved",
            "A reserved record must finish or enter recovery before Attempt close.",
            record_id=lock.get("record_id"),
        )

    _validate_execute_instruction_close_state(task, attempt, request)

    if request["status"] == "completed":
        _validate_completed_coverage(task=task, attempt=attempt)
    closed_attempt = build_closed_attempt(
        attempt,
        request,
        project_root=project_root,
        ended_at=_timestamp(now),
    )
    rendered_attempt = render_attempt_contract(
        closed_attempt, project_root=project_root
    )

    updated_index = build_closed_index(
        index,
        task_id=task_id,
        attempt_id=attempt_id,
        request=request,
    )
    task_status = updated_index["tasks"][
        next(
            position
            for position, item in enumerate(updated_index["tasks"])
            if item["id"] == task_id
        )
    ]["status"]
    rendered_index = render_execution_index(updated_index)
    validate_execution_index(rendered_index, source="generated attempt-close index")

    attempt_temporary = execution_path / (
        f".work-attempt-close-{task_id}-{attempt_id}-attempt.tmp"
    )
    index_temporary = execution_path / (
        f".work-attempt-close-{task_id}-{attempt_id}-index.tmp"
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
            stage="index_update_prepared",
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
        "schema": "work-attempt-close/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "attempt_path": attempt_relative,
        "index_path": index_relative,
        "attempt_status": request["status"],
        "task_status": task_status,
        "overall_status": updated_index["overall_status"],
        "lock_status": "released",
    }
