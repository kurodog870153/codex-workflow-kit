from __future__ import annotations

import re
from datetime import datetime
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import (
    parse_json_contract,
    parse_markdown_json_contract,
    render_markdown_json_contract,
    require_canonical_markdown_json_contract,
)
from ..foundation.paths import (
    normalize_relative_path,
    portable_path_identity,
    resolve_project_relative_path,
)


ATTEMPT_PATTERN = re.compile(r"^ATTEMPT-(\d{3})$")
TASK_PATTERN = re.compile(r"^TASK-\d{3}$")
TASK_SPEC_PATTERN = re.compile(r"^TASK-SPEC-\d{3}$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TIMESTAMP_PATTERN = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}[+-]\d{2}:\d{2}$"
)
RECORD_PATTERN = re.compile(r"^(CMD|OP|VAL)-\d{3}(?:#([1-9]\d*))?$")
ATTEMPT_STATUSES = {"in_progress", "completed", "stopped", "blocked"}
STOPPED_TYPES = {
    "specification_defect",
    "instructions_changed",
    "validation_failed",
    "unexpected_change",
    "external_operation_failed",
    "user_stopped",
    "other",
}
BLOCKED_TYPES = {
    "environment",
    "external_service",
    "permission",
    "required_input",
    "other",
}
ROOT_REQUIRED = {
    "schema",
    "attempt_id",
    "task_spec_id",
    "task_id",
    "skill_id",
    "status",
    "task_sha256",
    "task_instructions_sha256",
    "execute_instructions_sha256",
    "hierarchy_selection_sha256",
    "execute_skill_selection_sha256",
    "started_at",
    "records",
}
ROOT_OPTIONAL = {
    "continued_from",
    "carried_records",
    "modified_files",
    "overall_result",
    "final_type",
    "reason",
    "ended_at",
}
ROOT_ORDER = (
    "schema",
    "attempt_id",
    "task_spec_id",
    "task_id",
    "skill_id",
    "status",
    "task_sha256",
    "task_instructions_sha256",
    "execute_instructions_sha256",
    "hierarchy_selection_sha256",
    "execute_skill_selection_sha256",
    "started_at",
    "continued_from",
    "carried_records",
    "modified_files",
    "records",
    "overall_result",
    "final_type",
    "reason",
    "ended_at",
)


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, details or None)


def _strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
    optional: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        _fail("attempt_expected_object", "A JSON object is required.", location=location)
    allowed = required | (optional or set())
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - allowed)
    if missing or unknown:
        _fail(
            "attempt_invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            location=location,
            missing=missing,
            unknown=unknown,
        )
    return value


def _nonempty(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        _fail(
            "attempt_empty_text_value",
            "A non-empty string is required.",
            location=location,
        )
    return value


def _identifier(
    value: object, *, location: str, pattern: re.Pattern[str]
) -> str:
    identifier = _nonempty(value, location=location)
    if not pattern.fullmatch(identifier):
        _fail(
            "attempt_invalid_identifier",
            "An Attempt contract identifier has an invalid format.",
            location=location,
            value=identifier,
        )
    return identifier


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        _fail(
            "attempt_invalid_sha256",
            "A lowercase SHA-256 value is required.",
            location=location,
        )
    return value


def _timestamp(value: object, *, location: str) -> datetime:
    text = _nonempty(value, location=location)
    if not TIMESTAMP_PATTERN.fullmatch(text):
        _fail(
            "attempt_invalid_timestamp",
            "A minute-precision ISO 8601 timestamp with a numeric offset is required.",
            location=location,
            value=text,
        )
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "attempt_invalid_timestamp",
            "The Attempt timestamp is not a valid calendar time.",
            {"location": location, "value": text},
        ) from error
    if parsed.utcoffset() is None:
        _fail(
            "attempt_invalid_timestamp",
            "The Attempt timestamp must include a numeric offset.",
            location=location,
        )
    return parsed


def _record_identity(value: object, *, location: str) -> tuple[str, str, int]:
    record_id = _nonempty(value, location=location)
    match = RECORD_PATTERN.fullmatch(record_id)
    if not match:
        _fail(
            "attempt_invalid_record_id",
            "A record ID must use CMD-, OP-, or VAL- with an optional retry suffix.",
            location=location,
            value=record_id,
        )
    base_id = record_id.split("#", 1)[0]
    retry = int(match.group(2)) if match.group(2) else 0
    return record_id, base_id, retry


def _command_value(value: object, *, location: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        _fail("attempt_expected_object", "A JSON object is required.", location=location)
    mode = value.get("mode")
    if mode == "argv":
        command = _strict_object(
            value,
            location=location,
            required={"mode", "argv"},
        )
        argv = command["argv"]
        if not isinstance(argv, list) or not argv:
            _fail(
                "attempt_invalid_command_argv",
                "A command argv must be a non-empty string array.",
                location=f"{location}.argv",
            )
        canonical_argv = [
            _nonempty(item, location=f"{location}.argv[]") for item in argv
        ]
        return {"mode": "argv", "argv": canonical_argv}
    if mode == "shell":
        command = _strict_object(
            value,
            location=location,
            required={"mode", "script"},
        )
        return {
            "mode": "shell",
            "script": _nonempty(command["script"], location=f"{location}.script"),
        }
    _fail(
        "attempt_invalid_command_mode",
        "A command mode must be argv or shell.",
        location=f"{location}.mode",
    )


def canonicalize_command_correction(
    value: object, *, location: str = "correction"
) -> dict[str, Any]:
    correction = _strict_object(
        value,
        location=location,
        required={
            "original_command",
            "actual_command",
            "reason",
            "authorization_evidence",
        },
    )
    original = _command_value(
        correction["original_command"], location=f"{location}.original_command"
    )
    actual = _command_value(
        correction["actual_command"], location=f"{location}.actual_command"
    )
    if original["mode"] != actual["mode"]:
        _fail(
            "attempt_command_correction_mode_mismatch",
            "An equivalent command correction must preserve the command mode.",
            location=location,
        )
    if original == actual:
        _fail(
            "attempt_command_correction_unchanged",
            "A command correction must change the command value.",
            location=location,
        )
    return {
        "original_command": original,
        "actual_command": actual,
        "reason": _nonempty(correction["reason"], location=f"{location}.reason"),
        "authorization_evidence": _nonempty(
            correction["authorization_evidence"],
            location=f"{location}.authorization_evidence",
        ),
    }


def _validate_carried_records(
    value: object, *, continued_from: str
) -> tuple[list[dict[str, Any]], dict[str, int]]:
    if not isinstance(value, list) or not value:
        _fail(
            "attempt_invalid_carried_records",
            "carried_records must be a non-empty array when present.",
        )
    result: list[dict[str, Any]] = []
    seen: set[str] = set()
    last_retry: dict[str, int] = {}
    for index, raw_record in enumerate(value):
        location = f"carried_records[{index}]"
        record = _strict_object(
            raw_record,
            location=location,
            required={"source_attempt_id", "record_id", "evidence"},
        )
        source_attempt = _identifier(
            record["source_attempt_id"],
            location=f"{location}.source_attempt_id",
            pattern=ATTEMPT_PATTERN,
        )
        if source_attempt != continued_from:
            _fail(
                "attempt_carried_source_mismatch",
                "Every carried record must use continued_from as its source Attempt.",
                location=location,
            )
        record_id, base_id, retry = _record_identity(
            record["record_id"], location=f"{location}.record_id"
        )
        if record_id in seen:
            _fail(
                "attempt_duplicate_carried_record",
                "A carried record ID cannot be repeated.",
                record_id=record_id,
            )
        seen.add(record_id)
        last_retry[base_id] = max(last_retry.get(base_id, -1), retry)
        result.append(
            {
                "source_attempt_id": source_attempt,
                "record_id": record_id,
                "evidence": _nonempty(
                    record["evidence"], location=f"{location}.evidence"
                ),
            }
        )
    return result, last_retry


def _validate_records(
    value: object, *, carried_retries: dict[str, int]
) -> tuple[list[dict[str, Any]], dict[str, list[str]]]:
    if not isinstance(value, list):
        _fail("attempt_invalid_records", "records must be an array.")
    result: list[dict[str, Any]] = []
    outcomes = {"success": [], "failure": [], "unknown": []}
    seen: set[str] = set()
    last_retry = dict(carried_retries)
    for index, raw_record in enumerate(value):
        location = f"records[{index}]"
        if not isinstance(raw_record, dict):
            _fail(
                "attempt_expected_object",
                "A JSON object is required.",
                location=location,
            )
        kind = raw_record.get("kind")
        field_spec = {
            "command": (
                {"id", "kind", "exit_code", "result"},
                {"correction"},
            ),
            "operation": ({"id", "kind", "outcome", "state"}, set()),
            "validation": ({"id", "kind", "outcome", "evidence"}, set()),
        }.get(kind)
        if field_spec is None:
            _fail(
                "attempt_invalid_record_kind",
                "A record kind must be command, operation, or validation.",
                location=f"{location}.kind",
            )
        required_fields, optional_fields = field_spec
        record = _strict_object(
            raw_record,
            location=location,
            required=required_fields,
            optional=optional_fields,
        )
        record_id, base_id, retry = _record_identity(
            record["id"], location=f"{location}.id"
        )
        expected_prefix = {
            "command": "CMD",
            "operation": "OP",
            "validation": "VAL",
        }[kind]
        if not record_id.startswith(f"{expected_prefix}-"):
            _fail(
                "attempt_record_kind_mismatch",
                "The record ID prefix does not match its kind.",
                record_id=record_id,
                kind=kind,
            )
        if record_id in seen:
            _fail(
                "attempt_duplicate_record",
                "A record ID cannot be repeated.",
                record_id=record_id,
            )
        expected_retry = last_retry.get(base_id, -1) + 1
        if retry != expected_retry:
            _fail(
                "attempt_record_retry_sequence",
                "Record retries must begin with the original ID and increase without gaps.",
                record_id=record_id,
                expected_retry=expected_retry,
            )
        seen.add(record_id)
        last_retry[base_id] = retry

        if kind == "command":
            exit_code = record["exit_code"]
            if isinstance(exit_code, bool) or not isinstance(exit_code, int):
                _fail(
                    "attempt_invalid_exit_code",
                    "A command exit_code must be an integer.",
                    location=f"{location}.exit_code",
                )
            canonical_record: dict[str, Any] = {
                "id": record_id,
                "kind": kind,
            }
            if "correction" in record:
                canonical_record["correction"] = canonicalize_command_correction(
                    record["correction"], location=f"{location}.correction"
                )
            canonical_record.update(
                {
                    "exit_code": exit_code,
                    "result": _nonempty(
                        record["result"], location=f"{location}.result"
                    ),
                }
            )
            result.append(canonical_record)
        elif kind == "operation":
            outcome = record["outcome"]
            if outcome not in outcomes:
                _fail(
                    "attempt_invalid_operation_outcome",
                    "An operation outcome must be success, failure, or unknown.",
                    location=f"{location}.outcome",
                )
            outcomes[outcome].append(record_id)
            result.append(
                {
                    "id": record_id,
                    "kind": kind,
                    "outcome": outcome,
                    "state": _nonempty(
                        record["state"], location=f"{location}.state"
                    ),
                }
            )
        else:
            outcome = record["outcome"]
            if outcome not in {"passed", "failed"}:
                _fail(
                    "attempt_invalid_validation_outcome",
                    "A validation outcome must be passed or failed.",
                    location=f"{location}.outcome",
                )
            result.append(
                {
                    "id": record_id,
                    "kind": kind,
                    "outcome": outcome,
                    "evidence": _nonempty(
                        record["evidence"], location=f"{location}.evidence"
                    ),
                }
            )
    return result, outcomes


def _record_id_array(value: object, *, location: str) -> list[str]:
    if not isinstance(value, list) or not value:
        _fail(
            "attempt_invalid_record_id_array",
            "A non-empty record ID array is required.",
            location=location,
        )
    result = [
        _record_identity(item, location=f"{location}[]")[0] for item in value
    ]
    if len(result) != len(set(result)):
        _fail(
            "attempt_duplicate_record_id",
            "Record IDs in an outcome array must be unique.",
            location=location,
        )
    return result


def _validate_overall_result(
    value: object, *, operation_outcomes: dict[str, list[str]]
) -> dict[str, Any]:
    result = _strict_object(
        value,
        location="overall_result",
        required={"status"},
        optional={"effective", "not_effective", "unknown"},
    )
    details = {
        "effective": _record_id_array(
            result["effective"], location="overall_result.effective"
        )
        if "effective" in result
        else [],
        "not_effective": _record_id_array(
            result["not_effective"], location="overall_result.not_effective"
        )
        if "not_effective" in result
        else [],
        "unknown": _record_id_array(
            result["unknown"], location="overall_result.unknown"
        )
        if "unknown" in result
        else [],
    }
    expected_details = {
        "effective": operation_outcomes["success"],
        "not_effective": operation_outcomes["failure"],
        "unknown": operation_outcomes["unknown"],
    }
    if details != expected_details:
        _fail(
            "attempt_overall_result_mismatch",
            "Overall-result details must exactly classify every operation record.",
        )
    if details["unknown"]:
        expected_status = "uncertain_result"
    elif details["effective"] and details["not_effective"]:
        expected_status = "partial_success"
    elif details["not_effective"]:
        expected_status = "failure"
    else:
        expected_status = "complete_success"
    if result["status"] != expected_status:
        _fail(
            "attempt_overall_status_mismatch",
            "The overall result status conflicts with operation outcomes.",
            expected=expected_status,
            actual=result["status"],
        )
    canonical: dict[str, Any] = {"status": expected_status}
    for field in ("effective", "not_effective", "unknown"):
        if details[field]:
            canonical[field] = details[field]
    return canonical


def canonicalize_attempt_contract(
    contract: dict[str, Any], *, project_root: Path
) -> dict[str, Any]:
    _strict_object(
        contract,
        location="attempt",
        required=ROOT_REQUIRED,
        optional=ROOT_OPTIONAL,
    )
    if contract["schema"] != "work-attempt/v1":
        _fail("attempt_invalid_schema", "The Attempt schema is invalid.")
    attempt_id = _identifier(
        contract["attempt_id"], location="attempt_id", pattern=ATTEMPT_PATTERN
    )
    _identifier(
        contract["task_spec_id"],
        location="task_spec_id",
        pattern=TASK_SPEC_PATTERN,
    )
    _identifier(contract["task_id"], location="task_id", pattern=TASK_PATTERN)
    if contract["skill_id"] is not None:
        _nonempty(contract["skill_id"], location="skill_id")
    for field in (
        "task_sha256",
        "task_instructions_sha256",
        "execute_instructions_sha256",
        "hierarchy_selection_sha256",
        "execute_skill_selection_sha256",
    ):
        _sha256(contract[field], location=field)
    started_at = _timestamp(contract["started_at"], location="started_at")

    status = contract["status"]
    if status not in ATTEMPT_STATUSES:
        _fail(
            "attempt_invalid_status",
            "Attempt status must be in_progress, completed, stopped, or blocked.",
        )

    continued_from: str | None = None
    carried_records: list[dict[str, Any]] = []
    carried_retries: dict[str, int] = {}
    if "continued_from" in contract:
        continued_from = _identifier(
            contract["continued_from"],
            location="continued_from",
            pattern=ATTEMPT_PATTERN,
        )
        current_match = ATTEMPT_PATTERN.fullmatch(attempt_id)
        source_match = ATTEMPT_PATTERN.fullmatch(continued_from)
        if int(source_match.group(1)) >= int(current_match.group(1)):
            _fail(
                "attempt_invalid_continuation",
                "continued_from must identify an earlier Attempt.",
            )
    if "carried_records" in contract:
        if continued_from is None:
            _fail(
                "attempt_missing_continuation",
                "carried_records requires continued_from.",
            )
        carried_records, carried_retries = _validate_carried_records(
            contract["carried_records"], continued_from=continued_from
        )

    modified_files: list[str] = []
    if "modified_files" in contract:
        raw_files = contract["modified_files"]
        if not isinstance(raw_files, list) or not raw_files:
            _fail(
                "attempt_invalid_modified_files",
                "modified_files must be a non-empty array when present.",
            )
        modified_files = [
            normalize_relative_path(item, field="modified_files[]")
            for item in raw_files
        ]
        if modified_files != raw_files:
            _fail(
                "attempt_noncanonical_modified_file",
                "Modified-file paths must use normalized project-relative form.",
            )
        path_identities: set[str] = set()
        for item in modified_files:
            _, resolved = resolve_project_relative_path(
                project_root, item, field="modified_files[]"
            )
            identity = portable_path_identity(resolved)
            if identity in path_identities:
                _fail(
                    "attempt_duplicate_modified_file",
                    "Modified-file paths must have unique portable path identities.",
                )
            path_identities.add(identity)

    records, operation_outcomes = _validate_records(
        contract["records"], carried_retries=carried_retries
    )
    has_operations = any(operation_outcomes.values())
    overall_result: dict[str, Any] | None = None
    if "overall_result" in contract:
        if not has_operations:
            _fail(
                "attempt_unexpected_overall_result",
                "overall_result requires at least one operation record.",
            )
        overall_result = _validate_overall_result(
            contract["overall_result"], operation_outcomes=operation_outcomes
        )
    elif has_operations and status != "in_progress":
        _fail(
            "attempt_missing_overall_result",
            "A closed Attempt with operation records requires overall_result.",
        )

    closing_fields = {"final_type", "reason", "ended_at"} & set(contract)
    if status == "in_progress":
        if closing_fields:
            _fail(
                "attempt_unexpected_closing_fields",
                "An in-progress Attempt cannot contain closing fields.",
                fields=sorted(closing_fields),
            )
    else:
        if "ended_at" not in contract:
            _fail(
                "attempt_missing_end_time",
                "A closed Attempt requires ended_at.",
            )
        ended_at = _timestamp(contract["ended_at"], location="ended_at")
        if ended_at < started_at:
            _fail(
                "attempt_end_before_start",
                "ended_at cannot be earlier than started_at.",
            )
        if status == "completed":
            unexpected = {"final_type", "reason"} & set(contract)
            if unexpected:
                _fail(
                    "attempt_unexpected_final_details",
                    "A completed Attempt cannot contain final_type or reason.",
                    fields=sorted(unexpected),
                )
            if (
                overall_result is not None
                and overall_result["status"] != "complete_success"
            ):
                _fail(
                    "attempt_incomplete_operation_result",
                    "A completed Attempt cannot have a partial, failed, or uncertain operation result.",
                )
        else:
            missing = {"final_type", "reason"} - set(contract)
            if missing:
                _fail(
                    "attempt_missing_final_details",
                    "A stopped or blocked Attempt requires final_type and reason.",
                    missing=sorted(missing),
                )
            allowed_types = STOPPED_TYPES if status == "stopped" else BLOCKED_TYPES
            if contract["final_type"] not in allowed_types:
                _fail(
                    "attempt_invalid_final_type",
                    "final_type is invalid for the Attempt status.",
                    status=status,
                    final_type=contract["final_type"],
                )
            _nonempty(contract["reason"], location="reason")

    canonical_values: dict[str, Any] = dict(contract)
    canonical_values["records"] = records
    if carried_records:
        canonical_values["carried_records"] = carried_records
    if modified_files:
        canonical_values["modified_files"] = modified_files
    if overall_result is not None:
        canonical_values["overall_result"] = overall_result
    return {
        field: canonical_values[field]
        for field in ROOT_ORDER
        if field in canonical_values
    }


def validate_attempt_contract(
    contract: dict[str, Any], *, project_root: Path
) -> dict[str, object]:
    canonical = canonicalize_attempt_contract(contract, project_root=project_root)
    return {
        "schema": "work-attempt-validation/v1",
        "attempt_id": canonical["attempt_id"],
        "task_spec_id": canonical["task_spec_id"],
        "task_id": canonical["task_id"],
        "status": canonical["status"],
        "record_count": len(canonical["records"]),
        "result": "valid",
    }


def render_attempt_contract(
    contract: dict[str, Any], *, project_root: Path
) -> bytes:
    canonical = canonicalize_attempt_contract(contract, project_root=project_root)
    return render_markdown_json_contract(canonical["attempt_id"], canonical)


def validate_attempt_json_contract(
    raw: bytes, *, source: str, project_root: Path
) -> dict[str, object]:
    contract = parse_json_contract(raw, source=source)
    return validate_attempt_contract(contract, project_root=project_root)


def render_attempt_json_contract(
    raw: bytes, *, source: str, project_root: Path
) -> dict[str, Any]:
    contract = parse_json_contract(raw, source=source)
    return canonicalize_attempt_contract(contract, project_root=project_root)


def validate_attempt_file(
    project_root: Path, raw_attempt_path: str
) -> dict[str, object]:
    normalized_path, attempt_path = resolve_project_relative_path(
        project_root, raw_attempt_path, field="attempt_path"
    )
    try:
        raw = attempt_path.read_bytes()
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "attempt_read_failed",
            "The Attempt document could not be read.",
            {"path": normalized_path},
        ) from error
    title, contract = parse_markdown_json_contract(raw, source=normalized_path)
    canonical = canonicalize_attempt_contract(contract, project_root=project_root)
    if title != canonical["attempt_id"]:
        _fail(
            "attempt_title_mismatch",
            "The Attempt title does not match attempt_id.",
        )
    if attempt_path.name != f"{canonical['attempt_id']}.md":
        _fail(
            "attempt_filename_mismatch",
            "The Attempt filename does not match attempt_id.",
        )
    if attempt_path.parent.name != canonical["task_id"]:
        _fail(
            "attempt_parent_task_mismatch",
            "The Attempt parent directory does not match task_id.",
        )
    require_canonical_markdown_json_contract(
        raw,
        title=canonical["attempt_id"],
        contract=canonical,
        source=normalized_path,
    )
    result = validate_attempt_contract(canonical, project_root=project_root)
    result["path"] = normalized_path
    return result
