from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ..contracts.attempt import validate_attempt_file
from ..contracts.record_models import RecordBeginContract
from ..infrastructure.atomic_replace import TransactionErrors, prepare_and_replace
from ..foundation.errors import ExitCode, WorkError
from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..services.skill_catalog import SkillRoot
from .context import read_contract, find_task_row, load_lifecycle_task_context, validate_execution_identity
from .instructions import validate_execute_instructions
from .records import BASE_RECORD_PATTERN, next_record_id, formal_record_kind


LOCK_UPDATE_ERRORS = TransactionErrors(
    transaction_present=(
        "record_begin_transaction_present",
        "A record-begin transaction already requires recovery.",
    ),
    prepare_failed=(
        "record_begin_prepare_failed",
        "The record-begin index update could not be prepared.",
    ),
    source_changed=(
        "record_begin_index_changed",
        "The execution index changed during record begin.",
    ),
    replace_failed=(
        "record_begin_replace_failed",
        "The prepared record-begin index could not be installed.",
    ),
    write_mismatch=(
        "record_begin_write_mismatch",
        "The stored execution index does not match the prepared bytes.",
    ),
)



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
    prepare_and_replace(
        source_path=index_path,
        source_bytes=index_raw,
        target_bytes=rendered,
        temporary_path=temporary_path,
        stage="lock_update_prepared",
        partial_stage="lock_update_partial",
        errors=LOCK_UPDATE_ERRORS,
        validate_stored=lambda stored: validate_execution_index(
            stored, source=str(index_path)
        ),
        mismatch_recovery_required=False,
    )


def begin_record(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    base_record_id: str,
    authorization_evidence: str | None = None,
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
    attempt_relative = f"{normalized_execution}/{task_id}/{attempt_id}/attempt.json"
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
    require_record_scope(attempt, base_record_id)
    retry_evidence = require_retry_evidence(record_id, authorization_evidence)
    updated_index = copy.deepcopy(index)
    updated_index["lock"]["record_id"] = record_id
    if retry_evidence is not None:
        updated_index["lock"]["retry_authorization_evidence"] = retry_evidence
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
    return RecordBeginContract.model_validate({
        "schema": "work-record-begin/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "base_record_id": base_record_id,
        "record_id": record_id,
        "record_kind": record_kind,
        "index_path": index_relative,
        "lock_status": "record_reserved",
    }).to_canonical_dict()
from .authorization import require_record_scope, require_retry_evidence
