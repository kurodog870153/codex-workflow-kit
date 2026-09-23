from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ...services.attempt.validation import render_attempt_contract, validate_attempt_file, canonicalize_command_correction
from ...models.execution.recovery import ExecutionRecoveryContract
from ...services.recovery.validation import parse_execution_recovery_request
from ...models.common.errors import ExitCode, WorkError
from .attempt_close import (
    build_closed_attempt,
    build_closed_index,
)
from ...services.command.formalization import formal_command
from ...services.attempt.completion import validate_completed_coverage
from .context import read_contract, find_task_row, load_lifecycle_task_context, validate_execution_identity
from ...services.record.sequencing import next_record_id, formal_record_kind
from .record_finish import build_finished_attempt
from ...services.attempt.validation import render_execution_index, validate_execution_index
from ...services.execution.recovery import read_raw, parse_json_contract, resolve_project_relative_path
from ...services.execution.recovery import (
    install_recovery_target as _install,
    prepare_recovery_target as _prepare,
)
from ...services.skill_catalog import SkillRoot
from ...services.recovery.state import (
    attempt_close_request as _close_request,
    finished_record_index as _finished_index,
)


TRANSACTIONS = {
    "record_begin",
    "command_correction",
    "record_finish",
    "attempt_close",
    "correction",
    "deviation_record",
}


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def _read_json_contract(path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = read_raw(path)
    contract = parse_json_contract(raw, source=str(path))
    return raw, contract


def _validate_attempt_bytes(raw: bytes, *, project_root: Path, source: str) -> dict[str, Any]:
    contract = parse_json_contract(raw, source=source)
    if render_attempt_contract(contract, project_root=project_root) != raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_noncanonical_attempt",
            "The prepared Attempt bytes are not canonical.",
            source=source,
        )
    return contract


def _validate_index_bytes(raw: bytes, *, source: str) -> dict[str, Any]:
    validate_execution_index(raw, source=source)
    contract = parse_json_contract(raw, source=source)
    return contract


def _expected_files(execution_path: Path) -> list[str]:
    return sorted(path.name for path in execution_path.glob(".work-*.tmp") if path.is_file())


def _safe_record_id(record_id: str) -> str:
    return record_id.replace("#", "-retry-")


def _record_begin_recovery(
    *,
    execution_path: Path,
    index_path: Path,
    index_raw: bytes,
    index: dict[str, Any],
    attempt: dict[str, Any],
    task: dict[str, Any],
    task_id: str,
    attempt_id: str,
) -> dict[str, str]:
    lock = index["lock"]
    if "record_id" in lock or "command_correction" in lock or "retry_authorization_evidence" in lock:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_record_begin_state_conflict",
            "record_begin recovery requires an unreserved execution lock.",
        )
    candidates = list(execution_path.glob(f".work-record-begin-{task_id}-{attempt_id}-*.tmp"))
    if len(candidates) != 1:
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_record_begin_file_required",
            "record_begin recovery requires exactly one prepared index file.",
            files=sorted(path.name for path in candidates),
        )
    temporary = candidates[0]
    temporary_raw, target_index = _read_json_contract(temporary)
    validate_execution_index(temporary_raw, source=str(temporary))
    target_lock = target_index.get("lock")
    record_id = target_lock.get("record_id") if isinstance(target_lock, dict) else None
    if not isinstance(record_id, str):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_record_id_missing",
            "The prepared record_begin index does not reserve a record.",
        )
    expected_name = (
        f".work-record-begin-{task_id}-{attempt_id}-{_safe_record_id(record_id)}.tmp"
    )
    if temporary.name != expected_name:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_file_identity_mismatch",
            "The transaction file name does not match its prepared state.",
            expected=expected_name,
            actual=temporary.name,
        )
    base_record_id = record_id.split("#", 1)[0]
    formal_record_kind(task, base_record_id)
    if record_id != next_record_id(base_record_id, attempt):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_retry_sequence_mismatch",
            "The prepared record ID is not the next record instance.",
        )
    expected_index = copy.deepcopy(index)
    expected_index["lock"]["record_id"] = record_id
    expected_raw = render_execution_index(expected_index)
    if temporary_raw != expected_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_record_begin_target_mismatch",
            "The prepared record_begin index is not the unique canonical target.",
        )
    _install(
        temporary,
        index_path,
        expected=expected_raw,
        source_bytes=index_raw,
        stage="record_begin_lock_update",
    )
    validate_execution_index(read_raw(index_path), source=str(index_path))
    return {"record_id": record_id, "lock_status": "record_reserved"}


def _command_correction_recovery(
    *,
    execution_path: Path,
    index_path: Path,
    index_raw: bytes,
    index: dict[str, Any],
    task: dict[str, Any],
    task_id: str,
    attempt_id: str,
) -> dict[str, str]:
    lock = index["lock"]
    record_id = lock.get("record_id")
    if not isinstance(record_id, str) or not record_id.startswith("CMD-"):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_command_lock_required",
            "command_correction recovery requires a reserved CMD record.",
        )
    if "command_correction" in lock:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_command_correction_already_stored",
            "The command correction is already stored without a pending transaction file.",
        )
    expected_name = (
        f".work-command-correction-{task_id}-{attempt_id}-{_safe_record_id(record_id)}.tmp"
    )
    temporary = execution_path / expected_name
    if not temporary.is_file():
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_command_correction_file_required",
            "command_correction recovery requires its prepared index file.",
            expected=expected_name,
        )
    temporary_raw, target_index = _read_json_contract(temporary)
    validate_execution_index(temporary_raw, source=str(temporary))
    target_lock = target_index.get("lock")
    correction = (
        target_lock.get("command_correction")
        if isinstance(target_lock, dict)
        else None
    )
    correction = canonicalize_command_correction(
        correction, location="lock.command_correction"
    )
    base_record_id = record_id.split("#", 1)[0]
    if correction["original_command"] != formal_command(task, base_record_id):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_original_command_mismatch",
            "The prepared correction does not use the formal TASK command.",
        )
    expected_index = copy.deepcopy(index)
    expected_index["lock"]["command_correction"] = correction
    expected_raw = render_execution_index(expected_index)
    if temporary_raw != expected_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_command_correction_target_mismatch",
            "The prepared command_correction index is not the unique canonical target.",
        )
    _install(
        temporary,
        index_path,
        expected=expected_raw,
        source_bytes=index_raw,
        stage="command_correction_lock_update",
    )
    validate_execution_index(read_raw(index_path), source=str(index_path))
    return {"record_id": record_id, "lock_status": "record_reserved"}


def _deviation_record_recovery(
    *, project_root: Path, execution_path: Path, attempt_path: Path,
    attempt_relative: str, attempt_raw: bytes, attempt: dict[str, Any],
    index: dict[str, Any], task_id: str, attempt_id: str, **_unused: object,
) -> dict[str, str]:
    record_id = index["lock"].get("record_id")
    if not isinstance(record_id, str):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_record_lock_required",
            "deviation_record recovery requires a reserved record.",
        )
    temporary = execution_path / (
        f".work-deviation-record-{task_id}-{attempt_id}-{_safe_record_id(record_id)}-attempt.tmp"
    )
    current = list(attempt.get("execution_deviations", []))
    if temporary.exists():
        prepared_raw = read_raw(temporary)
        prepared = _validate_attempt_bytes(
            prepared_raw, project_root=project_root, source=str(temporary)
        )
        prepared_items = prepared.get("execution_deviations", [])
        if prepared_items[:-1] != current or len(prepared_items) != len(current) + 1:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_deviation_append_mismatch",
                "The prepared Attempt must append exactly one execution deviation.",
            )
        expected = copy.deepcopy(attempt)
        expected["execution_deviations"] = prepared_items
        expected_raw = render_attempt_contract(expected, project_root=project_root)
        if prepared_raw != expected_raw:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_deviation_target_mismatch",
                "The prepared deviation Attempt is not the unique canonical target.",
            )
        _install(
            temporary, attempt_path, expected=expected_raw,
            source_bytes=attempt_raw, stage="deviation_record_attempt_update",
        )
        validate_attempt_file(project_root, attempt_relative)
        current = prepared_items
    if not current:
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_deviation_missing",
            "No recorded or prepared execution deviation establishes recovery.",
        )
    return {"record_id": record_id, "lock_status": "record_reserved"}


def _record_finish_recovery(
    *,
    project_root: Path,
    execution_path: Path,
    attempt_path: Path,
    attempt_relative: str,
    attempt_raw: bytes,
    attempt: dict[str, Any],
    index_path: Path,
    index_raw: bytes,
    index: dict[str, Any],
    task: dict[str, Any],
    task_id: str,
    attempt_id: str,
) -> dict[str, str]:
    lock = index["lock"]
    record_id = lock.get("record_id")
    if not isinstance(record_id, str):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_record_lock_required",
            "record_finish recovery requires a reserved record.",
        )
    base_record_id = record_id.split("#", 1)[0]
    record_kind = formal_record_kind(task, base_record_id)
    safe_record = _safe_record_id(record_id)
    attempt_temporary = execution_path / (
        f".work-record-finish-{task_id}-{attempt_id}-{safe_record}-attempt.tmp"
    )
    index_temporary = execution_path / (
        f".work-record-finish-{task_id}-{attempt_id}-{safe_record}-index.tmp"
    )
    current_has_record = any(item["id"] == record_id for item in attempt["records"])
    if attempt_temporary.exists():
        if current_has_record:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_duplicate_attempt_target",
                "The current Attempt already contains the prepared record.",
            )
        prepared_raw = read_raw(attempt_temporary)
        prepared = _validate_attempt_bytes(
            prepared_raw, project_root=project_root, source=str(attempt_temporary)
        )
        if len(prepared["records"]) != len(attempt["records"]) + 1:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_record_append_mismatch",
                "The prepared Attempt must append exactly one record.",
            )
        result_record = copy.deepcopy(prepared["records"][-1])
        result_record.pop("correction", None)
        result_record.pop("id", None)
        result_record.pop("kind", None)
        finish_request: dict[str, Any] = {
            "schema": "work-record-finish-request/v1",
            "record": result_record,
        }
        if "modified_files" in prepared:
            finish_request["modified_files"] = prepared["modified_files"]
        expected_attempt = build_finished_attempt(
            attempt,
            finish_request,
            project_root=project_root,
            expected_record_id=record_id,
            expected_kind=record_kind,
            command_correction=lock.get("command_correction"),
        )
        expected_attempt_raw = render_attempt_contract(
            expected_attempt, project_root=project_root
        )
        if prepared_raw != expected_attempt_raw:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_record_finish_target_mismatch",
                "The prepared record_finish Attempt is not the unique canonical target.",
            )
        expected_index_raw = render_execution_index(_finished_index(index))
        _prepare(index_temporary, expected_index_raw)
        _install(
            attempt_temporary,
            attempt_path,
            expected=expected_attempt_raw,
            source_bytes=attempt_raw,
            stage="record_finish_attempt_update",
        )
        validate_attempt_file(project_root, attempt_relative)
        attempt = expected_attempt
        attempt_raw = expected_attempt_raw
        current_has_record = True
    if not current_has_record:
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_record_result_missing",
            "The record result is not preserved in an Attempt transaction target.",
        )
    expected_index_raw = render_execution_index(_finished_index(index))
    validate_execution_index(expected_index_raw, source="recovered record_finish index")
    _prepare(index_temporary, expected_index_raw)
    _install(
        index_temporary,
        index_path,
        expected=expected_index_raw,
        source_bytes=index_raw,
        stage="record_finish_index_update",
    )
    validate_execution_index(read_raw(index_path), source=str(index_path))
    return {"record_id": record_id, "lock_status": "attempt_held"}


def _attempt_close_recovery(
    *,
    project_root: Path,
    execution_path: Path,
    attempt_path: Path,
    attempt_relative: str,
    attempt_raw: bytes,
    attempt: dict[str, Any],
    index_path: Path,
    index_raw: bytes,
    index: dict[str, Any],
    task: dict[str, Any],
    task_id: str,
    attempt_id: str,
) -> dict[str, str]:
    attempt_temporary = execution_path / (
        f".work-attempt-close-{task_id}-{attempt_id}-attempt.tmp"
    )
    index_temporary = execution_path / (
        f".work-attempt-close-{task_id}-{attempt_id}-index.tmp"
    )
    if attempt_temporary.exists():
        if attempt["status"] != "in_progress":
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_duplicate_close_target",
                "The current Attempt is already closed while a close target remains.",
            )
        prepared_raw = read_raw(attempt_temporary)
        prepared = _validate_attempt_bytes(
            prepared_raw, project_root=project_root, source=str(attempt_temporary)
        )
        request = _close_request(prepared)
        if prepared["status"] == "completed":
            validate_completed_coverage(task=task, attempt=attempt)
        expected_attempt = build_closed_attempt(
            attempt,
            request,
            project_root=project_root,
            ended_at=prepared["ended_at"],
        )
        expected_attempt_raw = render_attempt_contract(
            expected_attempt, project_root=project_root
        )
        if prepared_raw != expected_attempt_raw:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_attempt_close_target_mismatch",
                "The prepared closed Attempt is not the unique canonical target.",
            )
        expected_index = build_closed_index(
            index,
            task_id=task_id,
            attempt_id=attempt_id,
            request=request,
        )
        expected_index_raw = render_execution_index(expected_index)
        _prepare(index_temporary, expected_index_raw)
        _install(
            attempt_temporary,
            attempt_path,
            expected=expected_attempt_raw,
            source_bytes=attempt_raw,
            stage="attempt_close_attempt_update",
        )
        validate_attempt_file(project_root, attempt_relative)
        attempt = expected_attempt
        attempt_raw = expected_attempt_raw
    elif attempt["status"] == "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_closed_attempt_missing",
            "The closed Attempt is not preserved in a transaction target.",
        )
    request = _close_request(attempt)
    expected_index = build_closed_index(
        index,
        task_id=task_id,
        attempt_id=attempt_id,
        request=request,
    )
    expected_index_raw = render_execution_index(expected_index)
    validate_execution_index(expected_index_raw, source="recovered attempt_close index")
    _prepare(index_temporary, expected_index_raw)
    _install(
        index_temporary,
        index_path,
        expected=expected_index_raw,
        source_bytes=index_raw,
        stage="attempt_close_index_update",
    )
    validate_execution_index(read_raw(index_path), source=str(index_path))
    return {
        "attempt_status": str(attempt["status"]),
        "lock_status": "released",
    }


def recover_execution(
    raw: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots: list[SkillRoot] | None = None, operations=None,
) -> dict[str, object]:
    request = parse_execution_recovery_request(
        raw, source=source
    ).to_canonical_dict()
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if not execution_path.is_dir():
        _error(
            ExitCode.IO_FAILURE,
            "execution_recovery_directory_missing",
            "The execution directory does not exist.",
            path=normalized_execution,
        )
    actual_files = _expected_files(execution_path)
    if actual_files != request["transaction_files"]:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_file_set_changed",
            "The transaction file set differs from the explicitly authorized state.",
            expected=request["transaction_files"],
            actual=actual_files,
        )
    if any(name.startswith(".work-attempt-start-") for name in actual_files):
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_attempt_start_requires_dedicated_command",
            "Attempt-start recovery requires recover-attempt-start.",
        )
    if request["transaction"] == "correction":
        from .correction import recover_correction

        return recover_correction(
            request,
            project_root=project_root,
            user_config_root=user_config_root,
            raw_task_path=raw_task_path,
            raw_execution_dir=raw_execution_dir,
            task_id=task_id,
            skill_roots=skill_roots,
            operations=operations,
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
            "execution_recovery_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
        )
    try:
        task = next(item for item in task_contract["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_task_not_found",
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
    attempt_id = request["attempt_id"]
    if row.get("latest_attempt") != attempt_id or row["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_active_attempt_mismatch",
            "Recovery requires the index active Attempt named by the request.",
            latest_attempt=row.get("latest_attempt"),
            status=row["status"],
        )
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}/attempt.json"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    attempt_raw, attempt = read_contract(attempt_path)
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
            "execution_recovery_lock_mismatch",
            "The execution lock does not match the recovery Attempt.",
            expected=expected_lock,
            actual=lock,
        )

    common = {
        "execution_path": execution_path,
        "index_path": index_path,
        "index_raw": index_raw,
        "index": index,
        "task": task,
        "task_id": task_id,
        "attempt_id": attempt_id,
    }
    transaction = request["transaction"]
    if transaction == "record_begin":
        details = _record_begin_recovery(attempt=attempt, **common)
    elif transaction == "command_correction":
        details = _command_correction_recovery(**common)
    elif transaction == "record_finish":
        details = _record_finish_recovery(
            project_root=project_root,
            attempt_path=attempt_path,
            attempt_relative=attempt_relative,
            attempt_raw=attempt_raw,
            attempt=attempt,
            **common,
        )
    elif transaction == "deviation_record":
        details = _deviation_record_recovery(
            project_root=project_root, attempt_path=attempt_path,
            attempt_relative=attempt_relative, attempt_raw=attempt_raw,
            attempt=attempt, **common,
        )
    else:
        details = _attempt_close_recovery(
            project_root=project_root,
            attempt_path=attempt_path,
            attempt_relative=attempt_relative,
            attempt_raw=attempt_raw,
            attempt=attempt,
            **common,
        )
    result: dict[str, object] = {
        "schema": "work-execution-recovery/v1",
        "transaction": transaction,
        "task_id": task_id,
        "attempt_id": attempt_id,
        "attempt_path": attempt_relative,
        "index_path": index_relative,
        "status": "recovered",
    }
    result.update(details)
    return ExecutionRecoveryContract.model_validate(result).to_canonical_dict()
