from __future__ import annotations

import re
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract


REQUEST_SCHEMA = "work-correction-create-request/v1"


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
