from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from .worktree import collect_git_status, worktree_snapshot_sha256


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def read_index(index_path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = read_raw(index_path)
    validate_execution_index(raw, source=str(index_path))
    contract = parse_json_contract(raw, source=str(index_path))
    return raw, contract


def transaction_path(
    execution_path: Path,
    *,
    task_id: str,
    attempt_id: str,
    stage: str,
) -> Path:
    return execution_path / (
        f".work-attempt-start-{task_id}-{attempt_id}-{stage}.tmp"
    )


def write_exclusive(path: Path, content: bytes, *, label: str) -> None:
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


def replace_index(
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
        write_exclusive(temporary_path, rendered, label="temporary execution index")
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


def transaction_stage(
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
        _, index = read_index(index_path)
    except WorkError:
        return "index_unreadable"
    if index.get("lock", {}).get("attempt_id") == attempt_id:
        return "lock_installed"
    if lock_temporary.exists():
        return "lock_index_prepared"
    return "not_started"


def raise_transaction_error(
    error: WorkError,
    *,
    index_path: Path,
    attempt_path: Path,
    lock_temporary: Path,
    started_temporary: Path,
    attempt_id: str,
) -> None:
    stage = transaction_stage(
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


def validate_snapshot(
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
