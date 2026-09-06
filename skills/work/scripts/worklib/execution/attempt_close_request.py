from __future__ import annotations

from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract


REQUEST_SCHEMA = "work-attempt-close-request/v1"


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_attempt_close_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "attempt_close_expected_object",
            "A JSON object is required.",
        )
    required = {"schema", "status"}
    optional = {"final_type", "reason"}
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required - optional)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_invalid_fields",
            "The attempt-close request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_invalid_schema",
            "The attempt-close request schema is invalid.",
        )
    status = request["status"]
    if status not in {"completed", "stopped", "blocked"}:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_invalid_status",
            "status must be completed, stopped, or blocked.",
            status=status,
        )
    final_fields = {"final_type", "reason"} & set(request)
    if status == "completed" and final_fields:
        _error(
            ExitCode.CONTRACT,
            "attempt_close_unexpected_final_details",
            "A completed Attempt cannot include final_type or reason.",
            fields=sorted(final_fields),
        )
    if status != "completed":
        missing_final = {"final_type", "reason"} - set(request)
        if missing_final:
            _error(
                ExitCode.CONTRACT,
                "attempt_close_missing_final_details",
                "A stopped or blocked Attempt requires final_type and reason.",
                missing=sorted(missing_final),
            )
        if any(
            not isinstance(request[field], str) or not request[field].strip()
            for field in ("final_type", "reason")
        ):
            _error(
                ExitCode.CONTRACT,
                "attempt_close_empty_final_detail",
                "final_type and reason must be non-empty strings.",
            )
    return request
