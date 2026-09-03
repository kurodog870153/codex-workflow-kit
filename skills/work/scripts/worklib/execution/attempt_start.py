from __future__ import annotations

import copy
import os
import re
from datetime import datetime
from pathlib import Path
from typing import Any

from ..contracts.attempt import (
    ATTEMPT_PATTERN,
    RECORD_PATTERN,
    canonicalize_attempt_contract,
    render_attempt_contract,
    validate_attempt_file,
)
from ..foundation.errors import ExitCode, WorkError
from .preflight import execute_preflight
from .worktree import (
    collect_git_status,
    inspect_execute_worktree,
    worktree_snapshot_sha256,
)
from ..contracts.execution_index import (
    derive_overall_status,
    render_execution_index,
    validate_execution_index,
)
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract, parse_markdown_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot


REQUEST_SCHEMA = "work-attempt-start-request/v1"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TRANSACTION_PATTERN = re.compile(
    r"^\.work-attempt-start-(TASK-\d{3})-(ATTEMPT-\d{3})-(lock|started)\.tmp$"
)


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def _strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
    optional: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        _error(
            ExitCode.CONTRACT,
            "attempt_start_expected_object",
            "A JSON object is required.",
            location=location,
        )
    allowed = required | (optional or set())
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - allowed)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "attempt_start_invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            location=location,
            missing=missing,
            unknown=unknown,
        )
    return value


def _nonempty(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        _error(
            ExitCode.CONTRACT,
            "attempt_start_empty_text_value",
            "A non-empty string is required.",
            location=location,
        )
    return value


def parse_attempt_start_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    request = _strict_object(
        request,
        location="attempt_start_request",
        required={"schema", "worktree_snapshot_sha256"},
        optional={"continuation"},
    )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "attempt_start_invalid_schema",
            "The Attempt-start request schema is invalid.",
        )
    snapshot = request["worktree_snapshot_sha256"]
    if not isinstance(snapshot, str) or not SHA256_PATTERN.fullmatch(snapshot):
        _error(
            ExitCode.CONTRACT,
            "attempt_start_invalid_worktree_snapshot",
            "A lowercase worktree snapshot SHA-256 is required.",
        )
    canonical: dict[str, Any] = {
        "schema": REQUEST_SCHEMA,
        "worktree_snapshot_sha256": snapshot,
    }
    if "continuation" in request:
        continuation = _strict_object(
            request["continuation"],
            location="continuation",
            required={"source_attempt_id", "carried_records"},
        )
        source_attempt = _nonempty(
            continuation["source_attempt_id"],
            location="continuation.source_attempt_id",
        )
        if not ATTEMPT_PATTERN.fullmatch(source_attempt):
            _error(
                ExitCode.CONTRACT,
                "attempt_start_invalid_source_attempt",
                "The continuation source Attempt ID is invalid.",
            )
        raw_records = continuation["carried_records"]
        if not isinstance(raw_records, list):
            _error(
                ExitCode.CONTRACT,
                "attempt_start_invalid_carried_records",
                "carried_records must be an array.",
            )
        carried_records: list[dict[str, str]] = []
        seen: set[str] = set()
        for position, raw_record in enumerate(raw_records):
            record = _strict_object(
                raw_record,
                location=f"continuation.carried_records[{position}]",
                required={"record_id", "evidence"},
            )
            record_id = _nonempty(
                record["record_id"],
                location=f"continuation.carried_records[{position}].record_id",
            )
            if not RECORD_PATTERN.fullmatch(record_id):
                _error(
                    ExitCode.CONTRACT,
                    "attempt_start_invalid_carried_record_id",
                    "A carried record ID is invalid.",
                    record_id=record_id,
                )
            if record_id in seen:
                _error(
                    ExitCode.CONTRACT,
                    "attempt_start_duplicate_carried_record",
                    "A carried record ID cannot be repeated.",
                    record_id=record_id,
                )
            seen.add(record_id)
            carried_records.append(
                {
                    "record_id": record_id,
                    "evidence": _nonempty(
                        record["evidence"],
                        location=(
                            f"continuation.carried_records[{position}].evidence"
                        ),
                    ),
                }
            )
        canonical["continuation"] = {
            "source_attempt_id": source_attempt,
            "carried_records": carried_records,
        }
    return canonical


def _read_index(index_path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = read_raw(index_path)
    validate_execution_index(raw, source=str(index_path))
    _, contract = parse_markdown_json_contract(raw, source=str(index_path))
    return raw, contract


def _task_row(index: dict[str, Any], task_id: str) -> dict[str, Any]:
    return next(item for item in index["tasks"] if item["id"] == task_id)


def _attempt_id_after(source_attempt_id: str | None) -> str:
    if source_attempt_id is None:
        return "ATTEMPT-001"
    match = ATTEMPT_PATTERN.fullmatch(source_attempt_id)
    assert match is not None
    number = int(match.group(1)) + 1
    if number > 999:
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_start_id_exhausted",
            "No additional three-digit Attempt ID is available.",
        )
    return f"ATTEMPT-{number:03d}"


def _validate_attempt_namespace(
    task_directory: Path,
    *,
    original_status: str,
    latest_attempt: str | None,
    attempt_id: str,
    allow_current: bool,
) -> None:
    existing = sorted(
        path.stem
        for path in task_directory.glob("ATTEMPT-*.md")
        if ATTEMPT_PATTERN.fullmatch(path.stem)
    ) if task_directory.is_dir() else []
    if original_status == "pending" and existing:
        allowed = [attempt_id] if allow_current else []
        if existing != allowed:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "attempt_start_unexpected_attempt_history",
                "An initial pending TASK has unexpected Attempt history.",
                attempts=existing,
            )
    if original_status == "pending_retry":
        if latest_attempt not in existing:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "attempt_start_latest_attempt_missing",
                "The latest Attempt file is missing.",
                latest_attempt=latest_attempt,
            )
        later = [
            item
            for item in existing
            if item != attempt_id
            and int(ATTEMPT_PATTERN.fullmatch(item).group(1))
            > int(ATTEMPT_PATTERN.fullmatch(str(latest_attempt)).group(1))
        ]
        if later or (attempt_id in existing and not allow_current):
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "attempt_start_attempt_history_conflict",
                "Attempt history conflicts with the next Attempt ID.",
                attempts=existing,
            )


def _load_source_attempt(
    *,
    project_root: Path,
    execution_dir: str,
    task_id: str,
    source_attempt_id: str,
) -> dict[str, Any]:
    raw_path = f"{execution_dir}/{task_id}/{source_attempt_id}.md"
    result = validate_attempt_file(project_root, raw_path)
    if result["status"] == "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_start_source_not_closed",
            "A continuation source Attempt must be closed.",
        )
    _, path = resolve_project_relative_path(
        project_root, raw_path, field="source_attempt_path"
    )
    _, contract = parse_markdown_json_contract(read_raw(path), source=str(path))
    return contract


def _build_attempt(
    *,
    project_root: Path,
    preflight: dict[str, object],
    index: dict[str, Any],
    request: dict[str, Any],
    original_status: str,
    attempt_id: str,
    started_at: str,
) -> dict[str, Any]:
    task_id = str(preflight["task_id"])
    row = _task_row(index, task_id)
    continuation = request.get("continuation")
    contract: dict[str, Any] = {
        "schema": "work-attempt/v1",
        "attempt_id": attempt_id,
        "task_spec_id": preflight["task_spec_id"],
        "task_id": task_id,
        "skill_id": preflight["skill_id"],
        "status": "in_progress",
        "task_sha256": preflight["task_sha256"],
        "task_instructions_sha256": preflight["task_instructions_sha256"],
        "execute_instructions_sha256": preflight["execute_instructions_sha256"],
        "hierarchy_selection_sha256": preflight["hierarchy_selection_sha256"],
        "execute_skill_selection_sha256": preflight["execute_skill_selection"]["selection_sha256"],
        "started_at": started_at,
        "records": [],
    }
    if original_status == "pending":
        if continuation is not None:
            _error(
                ExitCode.WORKFLOW_STATE,
                "attempt_start_unexpected_continuation",
                "An initial pending TASK cannot use continuation data.",
            )
        expected_attempt_id = _attempt_id_after(None)
    elif original_status == "pending_retry":
        if continuation is None:
            _error(
                ExitCode.CONTRACT,
                "attempt_start_continuation_required",
                "A pending_retry TASK requires continuation data.",
            )
        latest_attempt = row.get("latest_attempt")
        if latest_attempt is None:
            _error(
                ExitCode.WORKFLOW_STATE,
                "attempt_start_latest_attempt_required",
                "A pending_retry TASK must identify its latest Attempt.",
            )
        if continuation["source_attempt_id"] != latest_attempt:
            _error(
                ExitCode.CONTRACT,
                "attempt_start_continuation_source_mismatch",
                "The continuation source must be the latest Attempt.",
                expected=latest_attempt,
                actual=continuation["source_attempt_id"],
            )
        source_contract = _load_source_attempt(
            project_root=project_root,
            execution_dir=str(preflight["execution_dir"]),
            task_id=task_id,
            source_attempt_id=latest_attempt,
        )
        for field in (
            "skill_id",
            "hierarchy_selection_sha256",
            "execute_skill_selection_sha256",
        ):
            if source_contract[field] != contract[field]:
                _error(
                    ExitCode.ARTIFACT_INTEGRITY,
                    "attempt_start_continuation_skill_identity_mismatch",
                    "A continuation must preserve its source Attempt selection identity.",
                    field=field,
                    expected=source_contract[field],
                    actual=contract[field],
                )
        available_ids = {item["id"] for item in source_contract["records"]}
        available_ids.update(
            item["record_id"] for item in source_contract.get("carried_records", [])
        )
        requested_ids = {
            item["record_id"] for item in continuation["carried_records"]
        }
        unknown = sorted(requested_ids - available_ids)
        if unknown:
            _error(
                ExitCode.CONTRACT,
                "attempt_start_unknown_carried_record",
                "A requested carried record does not exist in the source Attempt.",
                record_ids=unknown,
            )
        contract["continued_from"] = latest_attempt
        if continuation["carried_records"]:
            contract["carried_records"] = [
                {
                    "source_attempt_id": latest_attempt,
                    "record_id": item["record_id"],
                    "evidence": item["evidence"],
                }
                for item in continuation["carried_records"]
            ]
        if source_contract.get("modified_files"):
            contract["modified_files"] = source_contract["modified_files"]
        expected_attempt_id = _attempt_id_after(latest_attempt)
    else:
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_start_invalid_original_status",
            "The original TASK status is not eligible for Attempt start.",
            status=original_status,
        )
    if attempt_id != expected_attempt_id:
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_start_id_mismatch",
            "The Attempt ID is not the next ID for the TASK.",
            expected=expected_attempt_id,
            actual=attempt_id,
        )
    return canonicalize_attempt_contract(contract, project_root=project_root)


def _timestamp(now: datetime | None) -> str:
    value = now or datetime.now().astimezone()
    if value.tzinfo is None or value.utcoffset() is None:
        _error(
            ExitCode.CONTRACT,
            "attempt_start_naive_time",
            "Attempt start time must include a timezone offset.",
        )
    return value.replace(second=0, microsecond=0).isoformat(timespec="minutes")


def _lock(
    *, task_id: str, attempt_id: str, execute_instructions_sha256: str
) -> dict[str, str]:
    return {
        "kind": "execution",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "execute_instructions_sha256": execute_instructions_sha256,
    }


def _locked_index(
    index: dict[str, Any], *, lock: dict[str, str]
) -> dict[str, Any]:
    result = copy.deepcopy(index)
    result["lock"] = lock
    return result


def _started_index(
    index: dict[str, Any], *, task_id: str, attempt_id: str
) -> dict[str, Any]:
    result = copy.deepcopy(index)
    row = _task_row(result, task_id)
    row["status"] = "in_progress"
    row["latest_attempt"] = attempt_id
    row.pop("status_reason", None)
    result["overall_status"] = derive_overall_status(
        [item["status"] for item in result["tasks"]]
    )
    return result


def _transaction_path(
    execution_path: Path,
    *,
    task_id: str,
    attempt_id: str,
    stage: str,
) -> Path:
    return execution_path / (
        f".work-attempt-start-{task_id}-{attempt_id}-{stage}.tmp"
    )


def _write_exclusive(path: Path, content: bytes, *, label: str) -> None:
    try:
        with path.open("xb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "attempt_start_target_exists",
            f"The {label} already exists.",
            {"path": str(path)},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "attempt_start_write_failed",
            f"The {label} could not be written.",
            {"path": str(path)},
        ) from error


def _replace_index(
    *,
    index_path: Path,
    expected_current: bytes,
    target: dict[str, Any],
    temporary_path: Path,
    allow_existing_temporary: bool,
) -> bytes:
    rendered = render_execution_index(target)
    validate_execution_index(
        rendered, source="generated attempt-start execution index"
    )
    if temporary_path.exists():
        if not allow_existing_temporary or read_raw(temporary_path) != rendered:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "attempt_start_temporary_conflict",
                "An Attempt-start temporary index conflicts with the expected state.",
                path=str(temporary_path),
            )
    else:
        _write_exclusive(temporary_path, rendered, label="temporary execution index")
    if read_raw(index_path) != expected_current:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_index_changed",
            "The execution index changed during Attempt start.",
        )
    try:
        os.replace(temporary_path, index_path)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "attempt_start_index_replace_failed",
            "The prepared execution index could not be installed.",
            {"temporary_path": str(temporary_path), "index_path": str(index_path)},
        ) from error
    stored = read_raw(index_path)
    validate_execution_index(stored, source=str(index_path))
    if stored != rendered:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_index_write_mismatch",
            "The stored execution index does not match the prepared bytes.",
        )
    return stored


def _transaction_stage(
    *,
    index_path: Path,
    attempt_path: Path,
    lock_temporary: Path,
    started_temporary: Path,
    attempt_id: str,
) -> str:
    if started_temporary.exists():
        return "started_index_prepared"
    if attempt_path.exists():
        return "attempt_created"
    try:
        _, index = _read_index(index_path)
    except WorkError:
        return "index_unreadable"
    if index.get("lock", {}).get("attempt_id") == attempt_id:
        return "lock_installed"
    if lock_temporary.exists():
        return "lock_index_prepared"
    return "not_started"


def _raise_transaction_error(
    error: WorkError,
    *,
    index_path: Path,
    attempt_path: Path,
    lock_temporary: Path,
    started_temporary: Path,
    attempt_id: str,
) -> None:
    stage = _transaction_stage(
        index_path=index_path,
        attempt_path=attempt_path,
        lock_temporary=lock_temporary,
        started_temporary=started_temporary,
        attempt_id=attempt_id,
    )
    if stage == "not_started":
        raise error
    details = dict(error.details)
    details.update(
        {
            "attempt_id": attempt_id,
            "recovery_required": True,
            "transaction_stage": stage,
        }
    )
    raise WorkError(error.exit_code, error.code, error.message, details) from error


def _validate_snapshot(
    *, project_root: Path, execution_dir: str, expected: str
) -> None:
    actual = worktree_snapshot_sha256(
        collect_git_status(project_root), execution_dir=execution_dir
    )
    if actual != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_worktree_snapshot_changed",
            "The Git worktree snapshot changed after review.",
            expected=expected,
            actual=actual,
        )


def _complete_transaction(
    *,
    project_root: Path,
    execution_path: Path,
    index_path: Path,
    index_raw: bytes,
    index: dict[str, Any],
    attempt: dict[str, Any],
    allow_recovery: bool,
) -> Path:
    task_id = attempt["task_id"]
    attempt_id = attempt["attempt_id"]
    lock = _lock(
        task_id=task_id,
        attempt_id=attempt_id,
        execute_instructions_sha256=attempt["execute_instructions_sha256"],
    )
    lock_temporary = _transaction_path(
        execution_path,
        task_id=task_id,
        attempt_id=attempt_id,
        stage="lock",
    )
    started_temporary = _transaction_path(
        execution_path,
        task_id=task_id,
        attempt_id=attempt_id,
        stage="started",
    )
    current_lock = index.get("lock")
    if current_lock is None:
        locked = _locked_index(index, lock=lock)
        index_raw = _replace_index(
            index_path=index_path,
            expected_current=index_raw,
            target=locked,
            temporary_path=lock_temporary,
            allow_existing_temporary=allow_recovery,
        )
        index = locked
    elif current_lock != lock:
        _error(
            ExitCode.LOCK_CONFLICT,
            "attempt_start_lock_conflict",
            "The execution lock does not match the Attempt-start transaction.",
            lock=current_lock,
        )

    task_directory = execution_path / task_id
    try:
        task_directory.mkdir(exist_ok=True)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "attempt_start_task_directory_failed",
            "The TASK execution directory could not be created.",
            {"path": str(task_directory)},
        ) from error
    attempt_path = task_directory / f"{attempt_id}.md"
    rendered_attempt = render_attempt_contract(attempt, project_root=project_root)
    if attempt_path.exists():
        if not allow_recovery or read_raw(attempt_path) != rendered_attempt:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "attempt_start_attempt_conflict",
                "The Attempt document conflicts with the expected contract.",
                path=str(attempt_path),
            )
    else:
        _write_exclusive(attempt_path, rendered_attempt, label="Attempt document")
    validate_attempt_file(
        project_root,
        attempt_path.relative_to(project_root).as_posix(),
    )

    row = _task_row(index, task_id)
    if row["status"] != "in_progress":
        started = _started_index(index, task_id=task_id, attempt_id=attempt_id)
        index_raw = _replace_index(
            index_path=index_path,
            expected_current=index_raw,
            target=started,
            temporary_path=started_temporary,
            allow_existing_temporary=allow_recovery,
        )
    elif row.get("latest_attempt") != attempt_id:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_index_attempt_conflict",
            "The in-progress TASK points to a different Attempt.",
        )
    validate_execution_index(read_raw(index_path), source=str(index_path))
    return attempt_path


def _paths(
    project_root: Path, raw_execution_dir: str
) -> tuple[str, Path, str, Path]:
    execution_dir, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    index_relative = f"{execution_dir}/index.md"
    _, index_path = resolve_project_relative_path(
        project_root, index_relative, field="execution_index"
    )
    return execution_dir, execution_path, index_relative, index_path


def start_attempt(
    raw_request: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    confirmed_inputs: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None,
    now: datetime | None = None,
) -> dict[str, object]:
    request = parse_attempt_start_request(raw_request, source=source)
    worktree = inspect_execute_worktree(
        project_root=project_root,
        user_config_root=user_config_root,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        task_id=task_id,
        confirmed_inputs=confirmed_inputs,
        skill_roots=skill_roots,
    )
    if worktree["snapshot_sha256"] != request["worktree_snapshot_sha256"]:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_worktree_snapshot_changed",
            "The Git worktree snapshot changed after review.",
            expected=request["worktree_snapshot_sha256"],
            actual=worktree["snapshot_sha256"],
        )
    execution_dir, execution_path, index_relative, index_path = _paths(
        project_root, raw_execution_dir
    )
    index_raw, index = _read_index(index_path)
    row = _task_row(index, task_id)
    original_status = row["status"]
    source_attempt = row.get("latest_attempt") if original_status == "pending_retry" else None
    attempt_id = _attempt_id_after(source_attempt)
    task_directory = execution_path / task_id
    _validate_attempt_namespace(
        task_directory,
        original_status=original_status,
        latest_attempt=source_attempt,
        attempt_id=attempt_id,
        allow_current=False,
    )
    attempt = _build_attempt(
        project_root=project_root,
        preflight=worktree,
        index=index,
        request=request,
        original_status=original_status,
        attempt_id=attempt_id,
        started_at=_timestamp(now),
    )
    attempt_path = task_directory / f"{attempt_id}.md"
    lock_temporary = _transaction_path(
        execution_path, task_id=task_id, attempt_id=attempt_id, stage="lock"
    )
    started_temporary = _transaction_path(
        execution_path, task_id=task_id, attempt_id=attempt_id, stage="started"
    )
    try:
        stored_attempt_path = _complete_transaction(
            project_root=project_root,
            execution_path=execution_path,
            index_path=index_path,
            index_raw=index_raw,
            index=index,
            attempt=attempt,
            allow_recovery=False,
        )
    except WorkError as error:
        _raise_transaction_error(
            error,
            index_path=index_path,
            attempt_path=attempt_path,
            lock_temporary=lock_temporary,
            started_temporary=started_temporary,
            attempt_id=attempt_id,
        )
    return {
        "schema": "work-attempt-start/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "attempt_path": stored_attempt_path.relative_to(project_root).as_posix(),
        "index_path": index_relative,
        "status": "started",
        "lock_status": "held",
    }


def _recovery_candidate(
    *, execution_path: Path, index: dict[str, Any], task_id: str
) -> str:
    lock = index.get("lock")
    if isinstance(lock, dict) and lock.get("kind") == "execution":
        if lock.get("task_id") != task_id:
            _error(
                ExitCode.LOCK_CONFLICT,
                "attempt_start_recovery_lock_task_mismatch",
                "The execution lock belongs to a different TASK.",
                lock=lock,
            )
        return str(lock["attempt_id"])
    candidates = {
        match.group(2)
        for path in execution_path.glob(".work-attempt-start-*.tmp")
        if (match := TRANSACTION_PATTERN.fullmatch(path.name))
        and match.group(1) == task_id
    }
    if len(candidates) != 1:
        _error(
            ExitCode.WORKFLOW_STATE,
            "attempt_start_recovery_candidate_ambiguous",
            "Recovery requires exactly one matching Attempt-start transaction.",
            candidates=sorted(candidates),
        )
    return next(iter(candidates))


def recover_attempt_start(
    raw_request: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    confirmed_inputs: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None,
    now: datetime | None = None,
) -> dict[str, object]:
    request = parse_attempt_start_request(raw_request, source=source)
    execution_dir, execution_path, index_relative, index_path = _paths(
        project_root, raw_execution_dir
    )
    index_raw, index = _read_index(index_path)
    attempt_id = _recovery_candidate(
        execution_path=execution_path, index=index, task_id=task_id
    )
    original_status = (
        "pending_retry" if request.get("continuation") is not None else "pending"
    )
    allowed_lock = index.get("lock")
    preflight = execute_preflight(
        project_root=project_root,
        user_config_root=user_config_root,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        task_id=task_id,
        confirmed_inputs=confirmed_inputs,
        skill_roots=skill_roots,
        _allowed_lock=allowed_lock,
        _allow_attempt_start_transaction=True,
        _eligible_statuses={"pending", "pending_retry", "in_progress"},
        _rule_status=original_status,
    )
    expected_lock = _lock(
        task_id=task_id,
        attempt_id=attempt_id,
        execute_instructions_sha256=str(
            preflight["execute_instructions_sha256"]
        ),
    )
    if isinstance(allowed_lock, dict) and allowed_lock != expected_lock:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_recovery_lock_mismatch",
            "The execution lock does not match current deterministic evidence.",
            expected=expected_lock,
            actual=allowed_lock,
        )
    _validate_snapshot(
        project_root=project_root,
        execution_dir=execution_dir,
        expected=request["worktree_snapshot_sha256"],
    )
    task_directory = execution_path / task_id
    row = _task_row(index, task_id)
    latest_attempt = (
        request.get("continuation", {}).get("source_attempt_id")
        if original_status == "pending_retry"
        else None
    )
    _validate_attempt_namespace(
        task_directory,
        original_status=original_status,
        latest_attempt=latest_attempt,
        attempt_id=attempt_id,
        allow_current=True,
    )
    attempt_path = task_directory / f"{attempt_id}.md"
    existing_started_at: str | None = None
    if attempt_path.exists():
        validation = validate_attempt_file(
            project_root, attempt_path.relative_to(project_root).as_posix()
        )
        if validation["attempt_id"] != attempt_id:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "attempt_start_recovery_attempt_mismatch",
                "The existing Attempt does not match the recovery candidate.",
            )
        _, existing = parse_markdown_json_contract(
            read_raw(attempt_path), source=str(attempt_path)
        )
        existing_started_at = existing["started_at"]
    build_index = index
    if row["status"] == "in_progress":
        build_index = copy.deepcopy(index)
        build_row = _task_row(build_index, task_id)
        build_row["status"] = original_status
        if original_status == "pending":
            build_row.pop("latest_attempt", None)
        else:
            build_row["latest_attempt"] = latest_attempt
    attempt = _build_attempt(
        project_root=project_root,
        preflight=preflight,
        index=build_index,
        request=request,
        original_status=original_status,
        attempt_id=attempt_id,
        started_at=existing_started_at or _timestamp(now),
    )
    lock_temporary = _transaction_path(
        execution_path, task_id=task_id, attempt_id=attempt_id, stage="lock"
    )
    started_temporary = _transaction_path(
        execution_path, task_id=task_id, attempt_id=attempt_id, stage="started"
    )
    try:
        stored_attempt_path = _complete_transaction(
            project_root=project_root,
            execution_path=execution_path,
            index_path=index_path,
            index_raw=index_raw,
            index=index,
            attempt=attempt,
            allow_recovery=True,
        )
    except WorkError as error:
        _raise_transaction_error(
            error,
            index_path=index_path,
            attempt_path=attempt_path,
            lock_temporary=lock_temporary,
            started_temporary=started_temporary,
            attempt_id=attempt_id,
        )
    return {
        "schema": "work-attempt-start-recovery/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "attempt_path": stored_attempt_path.relative_to(project_root).as_posix(),
        "index_path": index_relative,
        "status": "recovered",
        "lock_status": "held",
    }
