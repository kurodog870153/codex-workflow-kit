from __future__ import annotations

import re
from datetime import datetime
from pathlib import Path
from typing import Any

from .attempt import validate_attempt_file
from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import (
    parse_json_contract,
    parse_markdown_json_contract,
    render_markdown_json_contract,
    require_canonical_markdown_json_contract,
)
from ..foundation.paths import resolve_project_relative_path


CORRECTION_PATTERN = re.compile(r"^(ATTEMPT-\d{3})-CORRECTION-(\d{3})$")
ATTEMPT_PATTERN = re.compile(r"^ATTEMPT-\d{3}$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TIMESTAMP_PATTERN = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}[+-]\d{2}:\d{2}$"
)
FIELD_ORDER = (
    "schema",
    "correction_id",
    "created_at",
    "target_attempt_id",
    "task_instructions_sha256",
    "execute_instructions_sha256",
    "field",
    "correct_value",
    "reason",
)


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, details or None)


def _text(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        _fail(
            "correction_empty_text_value",
            "A non-empty string is required.",
            location=location,
        )
    return value


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        _fail(
            "correction_invalid_sha256",
            "A lowercase SHA-256 value is required.",
            location=location,
        )
    return value


def canonicalize_correction_contract(contract: object) -> dict[str, Any]:
    if not isinstance(contract, dict):
        _fail("correction_expected_object", "A JSON object is required.")
    required = set(FIELD_ORDER)
    missing = sorted(required - set(contract))
    unknown = sorted(set(contract) - required)
    if missing or unknown:
        _fail(
            "correction_invalid_fields",
            "The Correction contract has missing or unknown fields.",
            missing=missing,
            unknown=unknown,
        )
    if contract["schema"] != "work-correction/v1":
        _fail("correction_invalid_schema", "The Correction schema is invalid.")
    correction_id = _text(contract["correction_id"], location="correction_id")
    match = CORRECTION_PATTERN.fullmatch(correction_id)
    target_attempt_id = _text(
        contract["target_attempt_id"], location="target_attempt_id"
    )
    if not match or not ATTEMPT_PATTERN.fullmatch(target_attempt_id):
        _fail(
            "correction_invalid_identifier",
            "Correction and target Attempt IDs must use canonical formats.",
        )
    if match.group(1) != target_attempt_id:
        _fail(
            "correction_target_mismatch",
            "correction_id must be scoped to target_attempt_id.",
        )
    created_at = _text(contract["created_at"], location="created_at")
    if not TIMESTAMP_PATTERN.fullmatch(created_at):
        _fail(
            "correction_invalid_timestamp",
            "created_at must use minute-precision ISO 8601 with a numeric offset.",
        )
    try:
        parsed = datetime.fromisoformat(created_at)
    except ValueError as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "correction_invalid_timestamp",
            "created_at is not a valid calendar time.",
            {"value": created_at},
        ) from error
    if parsed.utcoffset() is None:
        _fail(
            "correction_invalid_timestamp",
            "created_at must include a numeric offset.",
        )
    canonical = dict(contract)
    _sha256(
        canonical["task_instructions_sha256"],
        location="task_instructions_sha256",
    )
    _sha256(
        canonical["execute_instructions_sha256"],
        location="execute_instructions_sha256",
    )
    for field in ("field", "correct_value", "reason"):
        _text(canonical[field], location=field)
    return {field: canonical[field] for field in FIELD_ORDER}


def validate_correction_contract(contract: object) -> dict[str, object]:
    canonical = canonicalize_correction_contract(contract)
    return {
        "schema": "work-correction-validation/v1",
        "correction_id": canonical["correction_id"],
        "target_attempt_id": canonical["target_attempt_id"],
        "result": "valid",
    }


def render_correction_contract(contract: object) -> bytes:
    canonical = canonicalize_correction_contract(contract)
    return render_markdown_json_contract(canonical["correction_id"], canonical)


def validate_correction_json_contract(raw: bytes, *, source: str) -> dict[str, object]:
    return validate_correction_contract(parse_json_contract(raw, source=source))


def render_correction_json_contract(raw: bytes, *, source: str) -> dict[str, Any]:
    return canonicalize_correction_contract(parse_json_contract(raw, source=source))


def validate_correction_file(
    project_root: Path, raw_correction_path: str
) -> dict[str, object]:
    normalized, path = resolve_project_relative_path(
        project_root, raw_correction_path, field="correction_path"
    )
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "correction_read_failed",
            "The Correction document could not be read.",
            {"path": normalized},
        ) from error
    title, contract = parse_markdown_json_contract(raw, source=normalized)
    canonical = canonicalize_correction_contract(contract)
    if title != canonical["correction_id"]:
        _fail(
            "correction_title_mismatch",
            "The Correction title does not match correction_id.",
        )
    if path.name != f"{canonical['correction_id']}.md":
        _fail(
            "correction_filename_mismatch",
            "The Correction filename does not match correction_id.",
        )
    attempt_relative = str(
        (Path(normalized).parent / f"{canonical['target_attempt_id']}.md").as_posix()
    )
    attempt_validation = validate_attempt_file(project_root, attempt_relative)
    if attempt_validation["status"] == "in_progress":
        _fail(
            "correction_target_not_closed",
            "A Correction must target a closed Attempt.",
        )
    require_canonical_markdown_json_contract(
        raw,
        title=canonical["correction_id"],
        contract=canonical,
        source=normalized,
    )
    result = validate_correction_contract(canonical)
    result["path"] = normalized
    return result
