"""Durable preparation and installation primitives for execution recovery."""
from __future__ import annotations

import os
from pathlib import Path

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def prepare_recovery_target(path: Path, expected: bytes) -> None:
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


def install_recovery_target(
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
