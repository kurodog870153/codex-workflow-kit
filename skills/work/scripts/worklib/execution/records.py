from __future__ import annotations

import re
from typing import Any

from ..foundation.errors import ExitCode, WorkError


BASE_RECORD_PATTERN = re.compile(r"^(CMD|OP|VAL)-\d{3}$")
INSTANCE_RECORD_PATTERN = re.compile(
    r"^(CMD|OP|VAL)-\d{3}(?:#([1-9]\d*))?$"
)


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def next_record_id(base_record_id: str, attempt: dict[str, Any]) -> str:
    if not isinstance(base_record_id, str) or not BASE_RECORD_PATTERN.fullmatch(
        base_record_id
    ):
        _error(
            ExitCode.CONTRACT,
            "record_begin_invalid_base_record_id",
            "record_id must be a base CMD-, OP-, or VAL- identifier.",
            record_id=base_record_id,
        )
    instances = [
        item["record_id"] for item in attempt.get("carried_records", [])
    ] + [item["id"] for item in attempt.get("records", [])]
    retries: list[int] = []
    for record_id in instances:
        match = INSTANCE_RECORD_PATTERN.fullmatch(record_id)
        if match and record_id.split("#", 1)[0] == base_record_id:
            retries.append(int(match.group(2)) if match.group(2) else 0)
    if not retries:
        return base_record_id
    return f"{base_record_id}#{max(retries) + 1}"


def formal_record_kind(task: dict[str, Any], base_record_id: str) -> str:
    prefix = base_record_id[:3]
    field, kind = {
        "CMD": ("commands", "command"),
        "OP-": ("operations", "operation"),
        "VAL": ("validations", "validation"),
    }[prefix]
    if not any(item["id"] == base_record_id for item in task.get(field, [])):
        _error(
            ExitCode.CONTRACT,
            "record_begin_record_not_found",
            "The requested record ID is not defined by the target TASK.",
            record_id=base_record_id,
        )
    return kind
