from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ..contracts.attempt import validate_attempt_file
from ..contracts.command_correction import canonicalize_command_correction
from ..contracts.command_models import CommandCorrectionContract, CommandCorrectionRequestContract
from ..infrastructure.atomic_replace import TransactionErrors, prepare_and_replace
from ..foundation.errors import ExitCode, WorkError
from .context import read_contract, find_task_row, load_lifecycle_task_context, validate_execution_identity
from .instructions import validate_execute_instructions
from .records import next_record_id, formal_record_kind
from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..foundation.fingerprint import read_raw
from ..foundation.paths import resolve_project_relative_path
from ..services.skill_catalog import SkillRoot
from .commands import formal_command
from .authorization import authorization_evidence, require_deviation


LOCK_UPDATE_ERRORS = TransactionErrors(
    transaction_present=(
        "command_correction_transaction_present",
        "A command-correction transaction already requires recovery.",
    ),
    prepare_failed=(
        "command_correction_prepare_failed",
        "The command-correction index update could not be prepared.",
    ),
    source_changed=(
        "command_correction_index_changed",
        "The execution index changed during command correction.",
    ),
    replace_failed=(
        "command_correction_replace_failed",
        "The prepared command-correction index could not be installed.",
    ),
    write_mismatch=(
        "command_correction_stored_bytes_mismatch",
        "The installed command-correction index bytes are not canonical.",
    ),
)


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def _write_index_update(
    *,
    index_path: Path,
    index_raw: bytes,
    target_index: dict[str, Any],
    temporary_path: Path,
) -> None:
    rendered = render_execution_index(target_index)
    validate_execution_index(rendered, source="generated command-correction index")
    prepare_and_replace(
        source_path=index_path,
        source_bytes=index_raw,
        target_bytes=rendered,
        temporary_path=temporary_path,
        stage="lock_update_prepared",
        partial_stage="lock_update_partial",
        mismatch_stage="lock_updated",
        errors=LOCK_UPDATE_ERRORS,
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
    request = CommandCorrectionRequestContract.parse_request(
        raw, source=source
    ).to_execution_dict()
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

    normalized_task, task_path, task_contract, task_validation = load_lifecycle_task_context(
        raw_task_path=raw_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        task_id=task_id,
        skill_roots=skill_roots,
    )
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
            "command_correction_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            status=row["status"],
        )
    attempt_id = row["latest_attempt"]
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}/attempt.json"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    _, attempt = read_contract(attempt_path)
    if attempt["status"] != "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "command_correction_attempt_not_in_progress",
            "The latest Attempt is not in progress.",
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
    if formal_record_kind(task, base_record_id) != "command":
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
            "original_command": formal_command(task, base_record_id),
            "actual_command": request["correction"]["actual_command"],
            "reason": request["correction"]["reason"],
            "authorization_evidence": authorization_evidence(attempt, lock),
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
    action = {
        "kind": "replace_command",
        "record_id": record_id,
        "replacement": formal_command["actual_command"],
    }
    require_deviation(attempt, action)
    request["correction"] = formal_command

    validate_execute_instructions(
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
    return CommandCorrectionContract.model_validate({
        "schema": "work-command-correction/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "record_id": record_id,
        "index_path": index_relative,
        "correction_status": "recorded",
        "lock_status": "record_reserved",
    }).to_canonical_dict()
