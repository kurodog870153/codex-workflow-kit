"""File requests and the public Work CLI response envelope."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ...models.common.cli import CliResultContract, ErrorContract
from ...models.common.errors import ExitCode, WorkError
from .text_codec import decode_utf8
from ..foundation.jsonio import canonical_json


_BRIEF_FIELDS = frozenset({
    "schema",
    "status",
    "next_action",
    "command",
    "arguments",
    "semantic_input_contract",
    "requires_user_confirmation",
    "confirmation_required",
    "recovery_required",
    "required_checks",
    "routing_status",
    "router_compatibility_revision",
    "required_instruction_sources",
    "source_order",
    "routing_reasons",
    "selection_manifest",
    "path",
    "paths",
    "artifact",
    "artifacts",
    "target",
    "targets",
})


def _brief_field(name: str) -> bool:
    return (
        name in _BRIEF_FIELDS
        or name.endswith("_id")
        or name.endswith("_ids")
        or name.endswith("_sha256")
        or name.endswith("_path")
        or name.endswith("_paths")
    )


def brief_success_data(result: dict[str, Any]) -> dict[str, Any]:
    """Project a successful command result to continuation-critical fields."""

    def project(value: Any) -> Any:
        if not isinstance(value, dict):
            return value
        compact: dict[str, Any] = {}
        for key, item in value.items():
            if _brief_field(key):
                compact[key] = item
                continue
            if isinstance(item, dict):
                nested = project(item)
                if nested:
                    compact[key] = nested
        return compact

    return project(result)


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
    verbose: bool = False, full_evidence: bool = False,
) -> dict[str, Any]:
    selected = result if verbose or full_evidence else brief_success_data(result)
    data = selected if preserve_order else json.loads(canonical_json(selected))
    completed = result.get("status") == "already_completed"
    return CliResultContract(
        schema="work-cli-result/v1",
        status="already_completed" if completed else "success",
        reason_code="already_completed" if completed else "ok",
        message=(
            "The requested operation was already completed."
            if completed else "The command completed successfully."
        ),
        data=data,
    ).to_canonical_dict()


def error_response(error: WorkError) -> dict[str, Any]:
    detail = ErrorContract(
        schema="work-error/v1",
        code=error.code,
        message=error.message,
        details=error.details,
    )
    return CliResultContract(
        schema="work-cli-result/v1",
        status=(
            "failed"
            if error.exit_code in {ExitCode.IO_FAILURE, ExitCode.INTERNAL_ERROR}
            else "rejected"
        ),
        reason_code=detail.code,
        message=detail.message,
        data=detail.details,
    ).to_canonical_dict()
