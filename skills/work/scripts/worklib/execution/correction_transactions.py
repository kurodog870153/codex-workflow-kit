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


def prepare_file(path: Path, expected: bytes, *, stage: str) -> None:
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


def replace_file(
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


def install_exclusive(
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
    consume_temporary(temporary, stage=stage)


def consume_temporary(temporary: Path, *, stage: str) -> None:
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
