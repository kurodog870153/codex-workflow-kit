"""Durable storage primitives for Attempt-start transactions."""
from __future__ import annotations

import os
from pathlib import Path

from ...models.common.errors import ExitCode, WorkError
from .file_io import read_raw


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


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
    rendered: bytes,
    temporary_path: Path,
    allow_existing_temporary: bool,
) -> bytes:
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
    if stored != rendered:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "attempt_start_index_write_mismatch",
            "The stored execution index does not match the prepared bytes.",
        )
    return stored
