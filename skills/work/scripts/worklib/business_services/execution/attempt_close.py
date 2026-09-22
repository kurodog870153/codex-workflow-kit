from __future__ import annotations

import copy
from datetime import datetime
from pathlib import Path
from typing import Any

from ...protocol import BLOCKING_STOPPED_TYPES
from ...services.attempt.validation import (
    canonicalize_attempt_contract,
    render_attempt_contract,
    validate_attempt_file,
)
from ...models.execution.attempt_close import AttemptCloseContract
from ...services.attempt.close_validation import parse_attempt_close_request
from ...models.common.errors import ExitCode, WorkError
from .context import read_contract, find_task_row, load_lifecycle_task_context, validate_execution_identity
from .instructions import BASE_EXECUTE_REFERENCES, RECOVERY_REFERENCE
from ...services.attempt.validation import (
    derive_overall_status,
    render_execution_index,
    validate_execution_index,
)
from ...services.instruction.root import instruction_root
from ...services.execution.lifecycle import read_raw, parse_json_contract, resolve_project_relative_path
from ...services.skill_catalog import SkillRoot
from ...services.attempt.completion import validate_completed_coverage
from ...services.execution.lifecycle import TransactionErrors, prepare_and_replace
from ...services.attempt.state import close_index, closed_task_status
from ...services.deviation.validation import (
    deviation_is_blocking,
    deviation_reconciliation_target,
)


TRANSACTION_ERRORS = TransactionErrors(
    transaction_present=(
        "attempt_close_transaction_present",
        "An attempt-close transaction already requires recovery.",
    ),
    prepare_failed=(
        "attempt_close_prepare_failed",
        "The attempt-close update could not be prepared.",
    ),
    source_changed=(
        "attempt_close_source_changed",
        "A source artifact changed during Attempt close.",
    ),
    replace_failed=(
        "attempt_close_replace_failed",
        "The prepared attempt-close update could not be installed.",
    ),
    write_mismatch=(
        "attempt_close_stored_bytes_mismatch",
        "The installed attempt-close bytes do not match the prepared bytes.",
    ),
)
def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def _timestamp(now: datetime | None) -> str:
    value = now or datetime.now().astimezone()
    if value.tzinfo is None or value.utcoffset() is None:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_naive_time",
            "Attempt end time must include a timezone offset.",
        )
    return value.replace(second=0, microsecond=0).isoformat(timespec="minutes")


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
        candidate["closing_authorization_evidence"] = request["authorization_evidence"]
    candidate["ended_at"] = ended_at
    return canonicalize_attempt_contract(candidate, project_root=project_root)


def _task_status(request: dict[str, Any]) -> str:
    return closed_task_status(request)


def pending_deviation_summaries(attempt: dict[str, Any]) -> list[dict[str, object]]:
    return [
        {
            "deviation_id": deviation["deviation_id"],
            "classification": deviation_reconciliation_target(deviation["proposal"]),
            "blocking": deviation_is_blocking(deviation["proposal"]),
        }
        for deviation in attempt.get("execution_deviations", [])
        if deviation["decision"]["outcome"] == "approved"
        and deviation["reconciliation_status"] == "pending"
    ]


def has_blocking_deviation_for_record(
    attempt: dict[str, Any], record_id: object,
) -> bool:
    return any(
        deviation["decision"]["outcome"] == "approved"
        and deviation["reconciliation_status"] == "pending"
        and deviation_is_blocking(deviation["proposal"])
        and deviation["proposal"]["anchor_record_id"] == record_id
        for deviation in attempt.get("execution_deviations", [])
    )


def _validate_execute_instruction_close_state(
    task: dict[str, Any],
    attempt: dict[str, Any],
    request: dict[str, Any], operations,
) -> dict[str, object]:
    selection = task["instruction_selection"]
    references = list(BASE_EXECUTE_REFERENCES)
    if "continued_from" in attempt:
        references.append(RECOVERY_REFERENCE)
    current = operations.build_instruction_selection(
        skill_root=instruction_root(),
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
    statuses = [
        task_status if item["id"] == task_id else item["status"]
        for item in index["tasks"]
    ]
    return close_index(
        index,
        task_id=task_id,
        attempt_id=attempt_id,
        task_status=task_status,
        overall_status=derive_overall_status(statuses),
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
    operations=None,
) -> dict[str, object]:
    request = parse_attempt_close_request(raw, source=source).to_canonical_dict()
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

    normalized_task, task_path, task_contract, task_validation = load_lifecycle_task_context(
        raw_task_path=raw_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        task_id=task_id,
        skill_roots=skill_roots,
        operations=operations,
    )
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
            "attempt_close_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            status=row["status"],
        )
    attempt_id = row["latest_attempt"]
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}/attempt.json"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    attempt_raw, attempt = read_contract(attempt_path)
    if attempt["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_close_attempt_not_in_progress",
            "The latest Attempt is already closed.",
        )
    validate_execution_identity(
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
        reserved_record = lock.get("record_id")
        blocking_recorded = has_blocking_deviation_for_record(
            attempt, reserved_record
        )
        blocking_close = (
            request["status"] == "stopped"
            and request.get("final_type") == "specification_defect"
        )
        if "command_correction" in lock or not blocking_recorded or not blocking_close:
            _error(
                ExitCode.LOCK_CONFLICT,
                "attempt_close_record_reserved",
                "A reserved record may close only for its recorded blocking specification deviation.",
                record_id=reserved_record,
            )

    _validate_execute_instruction_close_state(task, attempt, request, operations)

    if request["status"] == "completed":
        validate_completed_coverage(task=task, attempt=attempt)
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
        prepare_and_replace(
            source_path=attempt_path,
            source_bytes=attempt_raw,
            target_bytes=rendered_attempt,
            temporary_path=attempt_temporary,
            stage="attempt_update_prepared",
            errors=TRANSACTION_ERRORS,
            mismatch_stage="attempt_update_updated",
        )
        validate_attempt_file(project_root, attempt_relative)
        prepare_and_replace(
            source_path=index_path,
            source_bytes=index_raw,
            target_bytes=rendered_index,
            temporary_path=index_temporary,
            stage="index_update_prepared",
            errors=TRANSACTION_ERRORS,
            mismatch_stage="index_update_updated",
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
    return AttemptCloseContract.model_validate({
        "schema": "work-attempt-close/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "attempt_path": attempt_relative,
        "index_path": index_relative,
        "attempt_status": request["status"],
        "task_status": task_status,
        "overall_status": updated_index["overall_status"],
        "pending_deviations": pending_deviation_summaries(closed_attempt),
        "lock_status": "released",
    }).to_canonical_dict()
