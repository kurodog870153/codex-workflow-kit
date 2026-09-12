"""File requests and the public Work CLI response envelope."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .errors import ExitCode, WorkError
from .fingerprint import decode_utf8
from .jsonio import canonical_json


@dataclass(frozen=True)
class FileInput:
    raw: bytes
    source: str


def read_input_file(value: str) -> FileInput:
    """Read once before dispatch; relative request paths use the process cwd."""
    path = Path(value)
    try:
        path = path.resolve(strict=True)
        if not path.is_file():
            raise OSError("The input must be a regular file.")
        raw = path.read_bytes()
    except (OSError, ValueError, RuntimeError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "input_file_read_failed",
            "The request file could not be read as a regular file.",
            {"path": value},
        ) from error
    source = str(path)
    text = decode_utf8(raw, source=source)
    if text.startswith("\ufeff"):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            {"source": source},
        )
    return FileInput(text.encode("utf-8"), source)


def success_response(
    result: dict[str, Any], *, preserve_order: bool = False,
) -> dict[str, Any]:
    data = result if preserve_order else json.loads(canonical_json(result))
    completed = result.get("status") == "already_completed"
    return {
        "schema": "work-cli-result/v1",
        "status": "already_completed" if completed else "success",
        "reason_code": "already_completed" if completed else "ok",
        "message": (
            "The requested operation was already completed."
            if completed else "The command completed successfully."
        ),
        "data": data,
    }


def error_response(error: WorkError) -> dict[str, Any]:
    return {
        "schema": "work-cli-result/v1",
        "status": (
            "failed" if error.exit_code in {ExitCode.IO_FAILURE, ExitCode.INTERNAL_ERROR}
            else "rejected"
        ),
        "reason_code": error.code,
        "message": error.message,
        "data": error.details,
    }
