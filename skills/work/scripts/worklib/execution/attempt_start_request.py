from __future__ import annotations

import re
from typing import Any

from ..contracts.attempt import ATTEMPT_PATTERN, RECORD_PATTERN
from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract


REQUEST_SCHEMA = "work-attempt-start-request/v1"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def _strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
    optional: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        _error(
            ExitCode.CONTRACT,
            "attempt_start_expected_object",
            "A JSON object is required.",
            location=location,
        )
    allowed = required | (optional or set())
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - allowed)
    if missing or unknown:
        _error(
            ExitCode.CONTRACT,
            "attempt_start_invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            location=location,
            missing=missing,
            unknown=unknown,
        )
    return value


def _nonempty(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        _error(
            ExitCode.CONTRACT,
            "attempt_start_empty_text_value",
            "A non-empty string is required.",
            location=location,
        )
    return value


def parse_attempt_start_request(raw: bytes, *, source: str) -> dict[str, Any]:
    request = parse_json_contract(raw, source=source)
    request = _strict_object(
        request,
        location="attempt_start_request",
        required={"schema", "worktree_snapshot_sha256"},
        optional={"continuation"},
    )
    if request["schema"] != REQUEST_SCHEMA:
        _error(
            ExitCode.CONTRACT,
            "attempt_start_invalid_schema",
            "The Attempt-start request schema is invalid.",
        )
    snapshot = request["worktree_snapshot_sha256"]
    if not isinstance(snapshot, str) or not SHA256_PATTERN.fullmatch(snapshot):
        _error(
            ExitCode.CONTRACT,
            "attempt_start_invalid_worktree_snapshot",
            "A lowercase worktree snapshot SHA-256 is required.",
        )
    canonical: dict[str, Any] = {
        "schema": REQUEST_SCHEMA,
        "worktree_snapshot_sha256": snapshot,
    }
    if "continuation" in request:
        continuation = _strict_object(
            request["continuation"],
            location="continuation",
            required={"source_attempt_id", "carried_records"},
        )
        source_attempt = _nonempty(
            continuation["source_attempt_id"],
            location="continuation.source_attempt_id",
        )
        if not ATTEMPT_PATTERN.fullmatch(source_attempt):
            _error(
                ExitCode.CONTRACT,
                "attempt_start_invalid_source_attempt",
                "The continuation source Attempt ID is invalid.",
            )
        raw_records = continuation["carried_records"]
        if not isinstance(raw_records, list):
            _error(
                ExitCode.CONTRACT,
                "attempt_start_invalid_carried_records",
                "carried_records must be an array.",
            )
        carried_records: list[dict[str, str]] = []
        seen: set[str] = set()
        for position, raw_record in enumerate(raw_records):
            record = _strict_object(
                raw_record,
                location=f"continuation.carried_records[{position}]",
                required={"record_id", "evidence"},
            )
            record_id = _nonempty(
                record["record_id"],
                location=f"continuation.carried_records[{position}].record_id",
            )
            if not RECORD_PATTERN.fullmatch(record_id):
                _error(
                    ExitCode.CONTRACT,
                    "attempt_start_invalid_carried_record_id",
                    "A carried record ID is invalid.",
                    record_id=record_id,
                )
            if record_id in seen:
                _error(
                    ExitCode.CONTRACT,
                    "attempt_start_duplicate_carried_record",
                    "A carried record ID cannot be repeated.",
                    record_id=record_id,
                )
            seen.add(record_id)
            carried_records.append(
                {
                    "record_id": record_id,
                    "evidence": _nonempty(
                        record["evidence"],
                        location=(
                            f"continuation.carried_records[{position}].evidence"
                        ),
                    ),
                }
            )
        canonical["continuation"] = {
            "source_attempt_id": source_attempt,
            "carried_records": carried_records,
        }
    return canonical
