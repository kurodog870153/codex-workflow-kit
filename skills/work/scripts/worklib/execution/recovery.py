from __future__ import annotations

import copy
import os
import re
from pathlib import Path
from typing import Any

from ..contracts.attempt import (
    canonicalize_command_correction,
    render_attempt_contract,
    validate_attempt_file,
)
from ..foundation.errors import ExitCode, WorkError
from .attempt_close import (
    _validate_completed_coverage,
    build_closed_attempt,
    build_closed_index,
)
from .command_correction import _formal_command
from .record_begin import (
    _formal_record,
    _read_contract,
    _task_row,
    _validate_identity,
    next_record_id,
)
from .record_finish import build_finished_attempt
from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract, parse_markdown_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot
from ..contracts.task import validate_task_contract


REQUEST_SCHEMA = "work-execution-recovery-request/v1"
TRANSACTIONS = {
    "record_begin",
    "command_correction",
    "record_finish",
    "attempt_close",
    "correction",
}
ATTEMPT_PATTERN = re.compile(r"^ATTEMPT-\d{3}$")


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_execution_recovery_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_expected_object",
            "A JSON object is required.",
        )
    required = {"schema", "transaction", "attempt_id", "transaction_files"}
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_invalid_fields",
            "The execution-recovery request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_invalid_schema",
            "The execution-recovery request schema is invalid.",
        )
    if request["transaction"] not in TRANSACTIONS:
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_invalid_transaction",
            "transaction is not supported by general execution recovery.",
            transaction=request["transaction"],
        )
    if not isinstance(request["attempt_id"], str) or not ATTEMPT_PATTERN.fullmatch(
        request["attempt_id"]
    ):
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_invalid_attempt_id",
            "attempt_id must use the canonical ATTEMPT-nnn format.",
        )
    files = request["transaction_files"]
    if not isinstance(files, list):
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_invalid_file_list",
            "transaction_files must be an array.",
        )
    if any(
        not isinstance(item, str)
        or not item
        or Path(item).name != item
        or "/" in item
        or "\\" in item
        for item in files
    ):
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_invalid_file_name",
            "Every transaction file must be a plain non-empty file name.",
        )
    if files != sorted(files) or len(files) != len(set(files)):
        _error(
            ExitCode.CONTRACT,
            "execution_recovery_noncanonical_file_list",
            "transaction_files must be unique and sorted.",
        )
    return request


def _read_markdown_contract(path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = read_raw(path)
    _, contract = parse_markdown_json_contract(raw, source=str(path))
    return raw, contract


def _validate_attempt_bytes(raw: bytes, *, project_root: Path, source: str) -> dict[str, Any]:
    _, contract = parse_markdown_json_contract(raw, source=source)
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
    _, contract = parse_markdown_json_contract(raw, source=source)
    return contract


def _prepare(path: Path, expected: bytes) -> None:
    if path.exists():
        if read_raw(path) != expected:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "execution_recovery_prepared_bytes_mismatch",
                "The prepared transaction bytes do not match the canonical target.",
                path=str(path),
            )
        return
    try:
        with path.open("xb") as output:
            output.write(expected)
            output.flush()
            os.fsync(output.fileno())
    except OSError as error:
        details: dict[str, object] = {"path": str(path)}
        if path.exists():
            details.update(
                {
                    "recovery_required": True,
                    "transaction_stage": "recovery_target_partial",
                }
            )
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execution_recovery_prepare_failed",
            "The canonical recovery target could not be prepared.",
            details,
        ) from error
    if read_raw(path) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            path=str(path),
            recovery_required=True,
            transaction_stage="recovery_target_prepared",
        )


def _install(
    temporary: Path,
    target: Path,
    *,
    expected: bytes,
    source_bytes: bytes,
    stage: str,
) -> None:
    if read_raw(temporary) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            path=str(temporary),
        )
    if read_raw(target) != source_bytes:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_source_changed",
            "A recovery source changed before replacement.",
            path=str(target),
        )
    try:
        os.replace(temporary, target)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execution_recovery_replace_failed",
            "The verified recovery target could not be installed.",
            {
                "path": str(temporary),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    if read_raw(target) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_recovery_stored_bytes_mismatch",
            "The installed recovery bytes do not match the verified target.",
            path=str(target),
            recovery_required=True,
            transaction_stage=stage,
        )


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
    if "record_id" in lock or "command_correction" in lock:
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
    temporary_raw, target_index = _read_markdown_contract(temporary)
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
    _formal_record(task, base_record_id)
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
    temporary_raw, target_index = _read_markdown_contract(temporary)
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
    if correction["original_command"] != _formal_command(task, base_record_id):
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


def _finished_index(index: dict[str, Any]) -> dict[str, Any]:
    target = copy.deepcopy(index)
    target["lock"].pop("record_id")
    target["lock"].pop("command_correction", None)
    return target


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
    record_kind = _formal_record(task, base_record_id)
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


def _close_request(attempt: dict[str, Any]) -> dict[str, Any]:
    request: dict[str, Any] = {
        "schema": "work-attempt-close-request/v1",
        "status": attempt["status"],
    }
    if attempt["status"] != "completed":
        request["final_type"] = attempt["final_type"]
        request["reason"] = attempt["reason"]
    return request


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
            _validate_completed_coverage(task=task, attempt=attempt)
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
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    request = parse_execution_recovery_request(raw, source=source)
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

    index_relative = f"{normalized_execution}/index.md"
    _, index_path = resolve_project_relative_path(
        project_root, index_relative, field="execution_index"
    )
    index_raw, index = _read_contract(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    row = _task_row(index, task_id)
    attempt_id = request["attempt_id"]
    if row.get("latest_attempt") != attempt_id or row["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "execution_recovery_active_attempt_mismatch",
            "Recovery requires the index active Attempt named by the request.",
            latest_attempt=row.get("latest_attempt"),
            status=row["status"],
        )
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}.md"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    attempt_raw, attempt = _read_contract(attempt_path)
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
    return result
