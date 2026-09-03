from __future__ import annotations

import copy
import os
import re
from datetime import datetime
from pathlib import Path
from typing import Any

from ..contracts.attempt import validate_attempt_file
from ..contracts.correction import (
    CORRECTION_PATTERN,
    canonicalize_correction_contract,
    render_correction_contract,
    validate_correction_file,
)
from ..foundation.errors import ExitCode, WorkError
from .record_begin import _read_contract, _task_row, _validate_identity
from ..contracts.execution_index import (
    derive_overall_status,
    render_execution_index,
    validate_execution_index,
)
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract, parse_markdown_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot
from ..contracts.task import validate_task_contract


REQUEST_SCHEMA = "work-correction-create-request/v1"
TRANSACTION_PATTERN = re.compile(
    r"^\.work-correction-(TASK-\d{3})-"
    r"(ATTEMPT-\d{3}-CORRECTION-\d{3})-(artifact|lock|index)\.tmp$"
)


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_correction_create_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "correction_create_expected_object",
            "A JSON object is required.",
        )
    required = {
        "schema",
        "target_attempt_id",
        "field",
        "correct_value",
        "reason",
        "invalidates_completion",
    }
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "correction_create_invalid_fields",
            "The Correction create request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "correction_create_invalid_schema",
            "The Correction create request schema is invalid.",
        )
    if not re.fullmatch(r"ATTEMPT-\d{3}", str(request["target_attempt_id"])):
        _error(
            ExitCode.CONTRACT,
            "correction_create_invalid_attempt_id",
            "target_attempt_id must use ATTEMPT-nnn.",
        )
    for field in ("field", "correct_value", "reason"):
        if not isinstance(request[field], str) or not request[field].strip():
            _error(
                ExitCode.CONTRACT,
                "correction_create_empty_text",
                "Correction text fields must be non-empty strings.",
                field=field,
            )
    if not isinstance(request["invalidates_completion"], bool):
        _error(
            ExitCode.CONTRACT,
            "correction_create_invalid_invalidation_flag",
            "invalidates_completion must be a boolean.",
        )
    return request


def _timestamp(now: datetime | None) -> str:
    value = now or datetime.now().astimezone()
    if value.tzinfo is None or value.utcoffset() is None:
        _error(
            ExitCode.CONTRACT,
            "correction_create_naive_time",
            "Correction time must include a timezone offset.",
        )
    return value.replace(second=0, microsecond=0).isoformat(timespec="minutes")


def _next_correction_id(task_path: Path, attempt_id: str) -> str:
    numbers: list[int] = []
    for path in task_path.glob(f"{attempt_id}-CORRECTION-*.md"):
        match = CORRECTION_PATTERN.fullmatch(path.stem)
        if not match or match.group(1) != attempt_id:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "correction_create_invalid_existing_name",
                "An existing Correction-like filename is not canonical.",
                path=str(path),
            )
        numbers.append(int(match.group(2)))
    number = max(numbers, default=0) + 1
    if number > 999:
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_create_id_exhausted",
            "The Correction ID range is exhausted for this Attempt.",
        )
    return f"{attempt_id}-CORRECTION-{number:03d}"


def _descendants(task_contract: dict[str, Any], task_id: str) -> set[str]:
    affected = {task_id}
    changed = True
    while changed:
        changed = False
        for task in task_contract["tasks"]:
            if task["id"] not in affected and any(
                dependency in affected for dependency in task.get("dependencies", [])
            ):
                affected.add(task["id"])
                changed = True
    return affected


def correction_affected_task_ids(
    index: dict[str, Any],
    task_contract: dict[str, Any],
    *,
    task_id: str,
    invalidates_completion: bool,
) -> list[str]:
    if not invalidates_completion:
        return []
    row = _task_row(index, task_id)
    if row["status"] != "completed":
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_create_target_not_completed",
            "Completion invalidation requires a completed target TASK.",
        )
    descendants = _descendants(task_contract, task_id)
    return [
        row["id"]
        for row in index["tasks"]
        if row["id"] in descendants and row["status"] == "completed"
    ]


def build_corrected_index(
    index: dict[str, Any],
    *,
    task_id: str,
    correction_id: str,
    affected_task_ids: list[str],
) -> dict[str, Any]:
    target = copy.deepcopy(index)
    target.pop("lock", None)
    target_row = _task_row(target, task_id)
    target_row["latest_correction"] = correction_id
    affected = set(affected_task_ids)
    for row in target["tasks"]:
        if row["id"] in affected:
            row["status"] = "pending_retry"
            row["status_reason"] = {"kind": "correction", "ref": correction_id}
    target["overall_status"] = derive_overall_status(
        [row["status"] for row in target["tasks"]]
    )
    return target


def _build_lock(
    *,
    task_id: str,
    attempt_id: str,
    correction_id: str,
    execute_instructions_sha256: str,
    invalidates_completion: bool,
    affected_task_ids: list[str],
) -> dict[str, Any]:
    return {
        "kind": "correction",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "correction_id": correction_id,
        "execute_instructions_sha256": execute_instructions_sha256,
        "invalidates_completion": invalidates_completion,
        "affected_task_ids": affected_task_ids,
    }


def _prepare(path: Path, expected: bytes, *, stage: str) -> None:
    try:
        with path.open("xb") as output:
            output.write(expected)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "correction_create_transaction_present",
            "A Correction transaction already requires recovery.",
            {
                "path": str(path),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    except OSError as error:
        details: dict[str, object] = {"path": str(path)}
        if path.exists():
            details.update(
                {"recovery_required": True, "transaction_stage": stage}
            )
        raise WorkError(
            ExitCode.IO_FAILURE,
            "correction_create_prepare_failed",
            "The Correction transaction target could not be prepared.",
            details,
        ) from error
    if read_raw(path) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_prepared_bytes_mismatch",
            "The prepared Correction transaction bytes are not canonical.",
            path=str(path),
            recovery_required=True,
            transaction_stage=stage,
        )


def _replace(
    temporary: Path,
    target: Path,
    *,
    expected: bytes,
    source_bytes: bytes | None,
    stage: str,
) -> None:
    if read_raw(temporary) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_prepared_bytes_mismatch",
            "The prepared Correction transaction bytes changed.",
            path=str(temporary),
        )
    if source_bytes is None:
        if target.exists():
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "correction_create_target_exists",
                "An immutable Correction target already exists.",
                path=str(target),
            )
    elif read_raw(target) != source_bytes:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_source_changed",
            "A Correction transaction source changed before replacement.",
            path=str(target),
        )
    try:
        os.replace(temporary, target)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "correction_create_replace_failed",
            "The prepared Correction transaction target could not be installed.",
            {
                "path": str(temporary),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    if read_raw(target) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_stored_bytes_mismatch",
            "The installed Correction transaction bytes do not match the target.",
            path=str(target),
            recovery_required=True,
            transaction_stage=stage,
        )


def _install_exclusive(
    temporary: Path,
    target: Path,
    *,
    expected: bytes,
    stage: str,
) -> None:
    if read_raw(temporary) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_prepared_bytes_mismatch",
            "The prepared immutable Correction bytes changed.",
            path=str(temporary),
        )
    if target.exists():
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_target_exists",
            "An immutable Correction target already exists.",
            path=str(target),
        )
    try:
        os.link(temporary, target)
    except FileExistsError as error:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_target_exists",
            "An immutable Correction target already exists.",
            {"path": str(target)},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "correction_create_exclusive_install_failed",
            "The immutable Correction could not be exclusively installed.",
            {
                "path": str(temporary),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    if read_raw(target) != expected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_create_stored_bytes_mismatch",
            "The installed immutable Correction bytes do not match the target.",
            path=str(target),
            recovery_required=True,
            transaction_stage=stage,
        )
    _consume_artifact_temporary(temporary, stage=stage)


def _consume_artifact_temporary(temporary: Path, *, stage: str) -> None:
    if not temporary.exists():
        return
    try:
        os.unlink(temporary)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "correction_create_temporary_consume_failed",
            "The installed Correction transaction file could not be consumed.",
            {
                "path": str(temporary),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error


def _load_context(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, Any]:
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
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
            "correction_create_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
        )
    index_relative = f"{normalized_execution}/index.md"
    _, index_path = resolve_project_relative_path(
        project_root, index_relative, field="execution_index"
    )
    index_raw, index = _read_contract(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    _task_row(index, task_id)
    return {
        "normalized_execution": normalized_execution,
        "execution_path": execution_path,
        "task_validation": task_validation,
        "task_contract": task_contract,
        "index_relative": index_relative,
        "index_path": index_path,
        "index_raw": index_raw,
        "index": index,
    }


def create_correction(
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
    request = parse_correction_create_request(raw, source=source)
    context = _load_context(
        project_root=project_root,
        user_config_root=user_config_root,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        task_id=task_id,
        skill_roots=skill_roots,
    )
    execution_path = context["execution_path"]
    if any(execution_path.glob(".work-*.tmp")):
        _error(
            ExitCode.LOCK_CONFLICT,
            "correction_create_transaction_present",
            "An execution transaction already requires recovery.",
        )
    index = context["index"]
    if "lock" in index:
        _error(
            ExitCode.LOCK_CONFLICT,
            "correction_create_lock_present",
            "A Correction requires an unlocked execution index.",
            lock=index["lock"],
        )
    attempt_id = request["target_attempt_id"]
    attempt_relative = (
        f"{context['normalized_execution']}/{task_id}/{attempt_id}.md"
    )
    validation = validate_attempt_file(project_root, attempt_relative)
    if validation["status"] == "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_create_attempt_not_closed",
            "A Correction must target a closed Attempt.",
        )
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    _, attempt = _read_contract(attempt_path)
    row = _validate_identity(
        task_contract=context["task_contract"],
        task_validation=context["task_validation"],
        index=index,
        attempt=attempt,
        task_id=task_id,
    )
    if request["invalidates_completion"] and row.get("latest_attempt") != attempt_id:
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_create_not_latest_attempt",
            "Only the latest Attempt can invalidate the current TASK completion.",
        )
    task_directory = attempt_path.parent
    correction_id = _next_correction_id(task_directory, attempt_id)
    affected_task_ids = correction_affected_task_ids(
        index,
        context["task_contract"],
        task_id=task_id,
        invalidates_completion=request["invalidates_completion"],
    )
    correction = canonicalize_correction_contract(
        {
            "schema": "work-correction/v1",
            "correction_id": correction_id,
            "created_at": _timestamp(now),
            "target_attempt_id": attempt_id,
            "task_instructions_sha256": attempt["task_instructions_sha256"],
            "execute_instructions_sha256": attempt[
                "execute_instructions_sha256"
            ],
            "field": request["field"],
            "correct_value": request["correct_value"],
            "reason": request["reason"],
        }
    )
    correction_raw = render_correction_contract(correction)
    lock = _build_lock(
        task_id=task_id,
        attempt_id=attempt_id,
        correction_id=correction_id,
        execute_instructions_sha256=attempt["execute_instructions_sha256"],
        invalidates_completion=request["invalidates_completion"],
        affected_task_ids=affected_task_ids,
    )
    locked_index = copy.deepcopy(index)
    locked_index["lock"] = lock
    locked_raw = render_execution_index(locked_index)
    validate_execution_index(locked_raw, source="generated Correction lock index")
    final_index = build_corrected_index(
        locked_index,
        task_id=task_id,
        correction_id=correction_id,
        affected_task_ids=affected_task_ids,
    )
    final_raw = render_execution_index(final_index)
    validate_execution_index(final_raw, source="generated corrected index")
    prefix = f".work-correction-{task_id}-{correction_id}"
    artifact_temporary = execution_path / f"{prefix}-artifact.tmp"
    lock_temporary = execution_path / f"{prefix}-lock.tmp"
    index_temporary = execution_path / f"{prefix}-index.tmp"
    correction_relative = (
        f"{context['normalized_execution']}/{task_id}/{correction_id}.md"
    )
    _, correction_path = resolve_project_relative_path(
        project_root, correction_relative, field="correction_path"
    )
    _prepare(artifact_temporary, correction_raw, stage="artifact_prepared")
    _prepare(lock_temporary, locked_raw, stage="lock_prepared")
    _prepare(index_temporary, final_raw, stage="index_prepared")
    _replace(
        lock_temporary,
        context["index_path"],
        expected=locked_raw,
        source_bytes=context["index_raw"],
        stage="lock_installed",
    )
    _install_exclusive(
        artifact_temporary,
        correction_path,
        expected=correction_raw,
        stage="correction_installed",
    )
    _replace(
        index_temporary,
        context["index_path"],
        expected=final_raw,
        source_bytes=locked_raw,
        stage="index_synchronized",
    )
    validate_correction_file(project_root, correction_relative)
    validate_execution_index(read_raw(context["index_path"]), source=context["index_relative"])
    return {
        "schema": "work-correction-create/v1",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "correction_id": correction_id,
        "correction_path": correction_relative,
        "index_path": context["index_relative"],
        "affected_task_ids": affected_task_ids,
        "lock_status": "released",
    }


def _transaction_identity(files: list[str], task_id: str) -> str:
    correction_ids: set[str] = set()
    for name in files:
        match = TRANSACTION_PATTERN.fullmatch(name)
        if not match or match.group(1) != task_id:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "correction_recovery_invalid_transaction_file",
                "Every recovery file must belong to one Correction transaction.",
                file=name,
            )
        correction_ids.add(match.group(2))
    if len(correction_ids) != 1:
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_recovery_identity_ambiguous",
            "Correction recovery requires exactly one transaction identity.",
            correction_ids=sorted(correction_ids),
        )
    return next(iter(correction_ids))


def recover_correction(
    request: dict[str, Any],
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    files = request["transaction_files"]
    context = _load_context(
        project_root=project_root,
        user_config_root=user_config_root,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        task_id=task_id,
        skill_roots=skill_roots,
    )
    index = context["index"]
    lock = index.get("lock")
    if isinstance(lock, dict) and lock.get("kind") == "correction":
        correction_id = lock["correction_id"]
    else:
        correction_id = _transaction_identity(files, task_id)
    if not CORRECTION_PATTERN.fullmatch(correction_id):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_invalid_id",
            "The preserved Correction ID is invalid.",
        )
    attempt_id = request["attempt_id"]
    if not correction_id.startswith(f"{attempt_id}-CORRECTION-"):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_attempt_mismatch",
            "The recovery request does not name the Correction target Attempt.",
        )
    prefix = f".work-correction-{task_id}-{correction_id}"
    expected_names = {
        f"{prefix}-artifact.tmp",
        f"{prefix}-lock.tmp",
        f"{prefix}-index.tmp",
    }
    if not set(files) <= expected_names:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_file_identity_mismatch",
            "The recovery file set contains another transaction.",
        )
    execution_path = context["execution_path"]
    artifact_temporary = execution_path / f"{prefix}-artifact.tmp"
    lock_temporary = execution_path / f"{prefix}-lock.tmp"
    index_temporary = execution_path / f"{prefix}-index.tmp"
    correction_relative = (
        f"{context['normalized_execution']}/{task_id}/{correction_id}.md"
    )
    _, correction_path = resolve_project_relative_path(
        project_root, correction_relative, field="correction_path"
    )
    if artifact_temporary.is_file():
        correction_raw = read_raw(artifact_temporary)
    elif correction_path.is_file():
        correction_raw = read_raw(correction_path)
    else:
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_recovery_artifact_missing",
            "The approved Correction content is not preserved.",
        )
    _, raw_correction = parse_markdown_json_contract(
        correction_raw, source="preserved Correction"
    )
    correction = canonicalize_correction_contract(raw_correction)
    if render_correction_contract(correction) != correction_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_noncanonical_artifact",
            "The preserved Correction bytes are not canonical.",
        )
    if correction["correction_id"] != correction_id:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_artifact_identity_mismatch",
            "The preserved Correction content has another identity.",
        )
    attempt_relative = (
        f"{context['normalized_execution']}/{task_id}/{attempt_id}.md"
    )
    attempt_validation = validate_attempt_file(project_root, attempt_relative)
    if attempt_validation["status"] == "in_progress":
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_recovery_attempt_not_closed",
            "Correction recovery requires a closed Attempt.",
        )
    _, attempt_path = resolve_project_relative_path(
        project_root, attempt_relative, field="attempt_path"
    )
    _, attempt = _read_contract(attempt_path)
    _validate_identity(
        task_contract=context["task_contract"],
        task_validation=context["task_validation"],
        index=index,
        attempt=attempt,
        task_id=task_id,
    )
    if (
        correction["task_instructions_sha256"]
        != attempt["task_instructions_sha256"]
        or correction["execute_instructions_sha256"]
        != attempt["execute_instructions_sha256"]
    ):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_fingerprint_mismatch",
            "The Correction fingerprints do not match the target Attempt.",
        )
    if isinstance(lock, dict) and lock.get("kind") == "correction":
        transaction_lock = lock
        base_index = copy.deepcopy(index)
        base_index.pop("lock")
    else:
        if not lock_temporary.is_file():
            _error(
                ExitCode.WORKFLOW_STATE,
                "correction_recovery_lock_target_missing",
                "The canonical Correction lock target is not preserved.",
            )
        lock_raw = read_raw(lock_temporary)
        validate_execution_index(lock_raw, source=str(lock_temporary))
        _, locked_contract = parse_markdown_json_contract(
            lock_raw, source=str(lock_temporary)
        )
        transaction_lock = locked_contract.get("lock")
        base_index = index
    if not isinstance(transaction_lock, dict):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_lock_missing",
            "The Correction lock metadata is missing.",
        )
    expected_lock_identity = {
        "kind": "correction",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "correction_id": correction_id,
        "execute_instructions_sha256": attempt["execute_instructions_sha256"],
    }
    if any(
        transaction_lock.get(field) != value
        for field, value in expected_lock_identity.items()
    ):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_lock_identity_mismatch",
            "The Correction lock does not match the preserved transaction.",
        )
    invalidates = transaction_lock.get("invalidates_completion")
    affected = transaction_lock.get("affected_task_ids")
    if not isinstance(invalidates, bool) or not isinstance(affected, list):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_lock_plan_invalid",
            "The Correction lock does not preserve a valid state plan.",
        )
    expected_affected = correction_affected_task_ids(
        base_index,
        context["task_contract"],
        task_id=task_id,
        invalidates_completion=invalidates,
    )
    if affected != expected_affected:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_affected_tasks_mismatch",
            "The preserved Correction state plan is no longer uniquely valid.",
        )
    expected_lock = _build_lock(
        task_id=task_id,
        attempt_id=attempt_id,
        correction_id=correction_id,
        execute_instructions_sha256=attempt["execute_instructions_sha256"],
        invalidates_completion=invalidates,
        affected_task_ids=affected,
    )
    locked_index = copy.deepcopy(base_index)
    locked_index["lock"] = expected_lock
    locked_raw = render_execution_index(locked_index)
    final_index = build_corrected_index(
        locked_index,
        task_id=task_id,
        correction_id=correction_id,
        affected_task_ids=affected,
    )
    final_raw = render_execution_index(final_index)
    current_has_lock = isinstance(lock, dict) and lock.get("kind") == "correction"
    if not current_has_lock:
        if not lock_temporary.is_file() or read_raw(lock_temporary) != locked_raw:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "correction_recovery_lock_bytes_mismatch",
                "The prepared Correction lock bytes are not the canonical target.",
            )
    elif context["index_raw"] != locked_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_current_lock_mismatch",
            "The current Correction lock index is not canonical.",
        )
    if correction_path.is_file():
        if read_raw(correction_path) != correction_raw:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "correction_recovery_existing_artifact_mismatch",
                "The immutable Correction target has conflicting bytes.",
            )
        if artifact_temporary.is_file() and read_raw(artifact_temporary) != correction_raw:
            _error(
                ExitCode.ARTIFACT_INTEGRITY,
                "correction_recovery_artifact_bytes_mismatch",
                "The preserved Correction transaction file has conflicting bytes.",
            )
    elif not artifact_temporary.is_file() or read_raw(artifact_temporary) != correction_raw:
        _error(
            ExitCode.WORKFLOW_STATE,
            "correction_recovery_artifact_target_missing",
            "The canonical Correction artifact target is not preserved.",
        )
    if not index_temporary.is_file() or read_raw(index_temporary) != final_raw:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "correction_recovery_index_target_mismatch",
            "The canonical final index target is not preserved.",
        )
    if not current_has_lock:
        _replace(
            lock_temporary,
            context["index_path"],
            expected=locked_raw,
            source_bytes=context["index_raw"],
            stage="recovery_lock_installed",
        )
    if correction_path.is_file():
        _consume_artifact_temporary(
            artifact_temporary, stage="recovery_correction_installed"
        )
    else:
        _install_exclusive(
            artifact_temporary,
            correction_path,
            expected=correction_raw,
            stage="recovery_correction_installed",
        )
    _replace(
        index_temporary,
        context["index_path"],
        expected=final_raw,
        source_bytes=locked_raw,
        stage="recovery_index_synchronized",
    )
    validate_correction_file(project_root, correction_relative)
    validate_execution_index(read_raw(context["index_path"]), source=context["index_relative"])
    return {
        "schema": "work-execution-recovery/v1",
        "transaction": "correction",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "correction_id": correction_id,
        "correction_path": correction_relative,
        "index_path": context["index_relative"],
        "affected_task_ids": affected,
        "lock_status": "released",
        "status": "recovered",
    }
