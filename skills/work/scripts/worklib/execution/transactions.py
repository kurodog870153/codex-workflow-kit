from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw


@dataclass(frozen=True)
class TransactionErrors:
    """Workflow-specific error codes and messages for a file replacement."""

    transaction_present: tuple[str, str]
    prepare_failed: tuple[str, str]
    source_changed: tuple[str, str]
    replace_failed: tuple[str, str]
    write_mismatch: tuple[str, str]


def prepare_and_replace(
    *,
    source_path: Path,
    source_bytes: bytes,
    target_bytes: bytes,
    temporary_path: Path,
    stage: str,
    errors: TransactionErrors,
    mismatch_stage: str | None = None,
) -> None:
    """Install prepared bytes, preserving failed writes for workflow recovery."""
    try:
        with temporary_path.open("xb") as output:
            output.write(target_bytes)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            *errors.transaction_present,
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
            *errors.prepare_failed,
            details,
        ) from error
    if read_raw(source_path) != source_bytes:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            *errors.source_changed,
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        )
    try:
        os.replace(temporary_path, source_path)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            *errors.replace_failed,
            {
                "path": str(temporary_path),
                "recovery_required": True,
                "transaction_stage": stage,
            },
        ) from error
    if read_raw(source_path) != target_bytes:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            *errors.write_mismatch,
            {
                "recovery_required": True,
                "transaction_stage": stage if mismatch_stage is None else mismatch_stage,
            },
        )
