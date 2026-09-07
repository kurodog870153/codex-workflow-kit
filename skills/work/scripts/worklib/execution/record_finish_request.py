from __future__ import annotations

from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract


REQUEST_SCHEMA = "work-record-finish-request/v1"


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def parse_record_finish_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    if not isinstance(request, dict):
        _error(
            ExitCode.CONTRACT,
            "record_finish_expected_object",
            "A JSON object is required.",
        )
    required = {"schema", "record"}
    optional = {"modified_files"}
    missing = sorted(required - set(request))
    unknown = sorted(set(request) - required - optional)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "record_finish_invalid_fields",
            "The record-finish request has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "record_finish_invalid_schema",
            "The record-finish request schema is invalid.",
        )
    if not isinstance(request["record"], dict):
        _error(
            ExitCode.CONTRACT,
            "record_finish_invalid_record",
            "record must be a JSON object.",
        )
    if "modified_files" in request:
        files = request["modified_files"]
        if not isinstance(files, list) or not files:
            _error(
                ExitCode.CONTRACT,
                "record_finish_invalid_modified_files",
                "modified_files must be a non-empty array when present.",
            )
        if any(not isinstance(item, str) or not item for item in files):
            _error(
                ExitCode.CONTRACT,
                "record_finish_invalid_modified_file",
                "Every modified file must be a non-empty string.",
            )
    return request
