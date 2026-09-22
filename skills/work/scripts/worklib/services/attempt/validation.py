from __future__ import annotations

import re
from datetime import datetime
from pathlib import Path
from typing import Any

from pydantic import ValidationError

from ...protocol import INVALID_SHA256_ERROR_CODE
from ...technical.foundation.fingerprint import canonical_json_sha256
from ...technical.infrastructure.json_contract import parse_json_contract, render_json_contract, require_canonical_json_contract
from ...technical.infrastructure.work_paths import (
    normalize_relative_path, portable_path_identity, resolve_project_relative_path,
    validate_execution_task_layout,
)
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.attempt import (
    ATTEMPT_PATTERN, ATTEMPT_STATUSES, BLOCKED_TYPES, RECORD_PATTERN, ROOT_ORDER,
    SHA256_PATTERN, STOPPED_TYPES, TASK_PATTERN, TASK_SPEC_PATTERN,
    TIMESTAMP_PATTERN, AttemptContract, AttemptValidationContract,
)
from ...models.execution.authorization import AttemptAuthorizationContract
from ...models.execution.deviation import (
    ExecutionDeviationContract, ExecutionDeviationImpactModel,
)


def _authorization_sha256(value: object) -> str:
    canonical = AttemptAuthorizationContract.model_validate(value).to_canonical_dict()
    return canonical_json_sha256(canonical)


def _attempt_model(contract: object) -> dict[str, Any]:
    try:
        return AttemptContract.model_validate(contract).to_canonical_dict()
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        missing = sorted(str(issue["loc"][-1]) for issue in issues if issue["type"] == "missing")
        unknown = sorted(str(issue["loc"][-1]) for issue in issues if issue["type"] == "extra_forbidden")
        if missing or unknown:
            _fail(
                "attempt_invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                location="attempt", missing=missing, unknown=unknown,
            )
        raise


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
        skipped = raw_record.get("status") == "skipped"
        field_spec = ({
            "command": ({"id", "kind", "status", "reason", "deviation_id"}, set()),
            "operation": ({"id", "kind", "status", "reason", "deviation_id"}, set()),
            "validation": ({"id", "kind", "status", "reason", "deviation_id"}, set()),
        } if skipped else {
            "command": (
                {"id", "kind", "exit_code", "result"},
                {"correction"},
            ),
            "operation": ({"id", "kind", "outcome", "state"}, set()),
            "validation": ({"id", "kind", "outcome", "evidence"}, set()),
        }).get(kind)
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

        if skipped:
            result.append({
                "id": record_id,
                "kind": kind,
                "status": "skipped",
                "reason": _nonempty(record["reason"], location=f"{location}.reason"),
                "deviation_id": _identifier(
                    record["deviation_id"],
                    location=f"{location}.deviation_id",
                    pattern=re.compile(r"^DEVIATION-[0-9]{3}$"),
                ),
            })
            continue

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


def _validate_skip_records(
    records: list[dict[str, Any]], deviations: list[dict[str, Any]], *, status: str,
) -> None:
    approved_skips = {
        item["proposal"]["action"]["record_id"]: item
        for item in deviations
        if item["decision"]["outcome"] == "approved"
        and item["proposal"]["action"]["kind"] == "skip_record"
        and not ExecutionDeviationImpactModel.crosses_semantic_boundary(
            item["proposal"]["impact"]
        )
    }
    recorded: set[str] = set()
    for record in records:
        deviation = approved_skips.get(record["id"])
        if record.get("status") == "skipped":
            if deviation is None:
                _fail(
                    "attempt_unapproved_skipped_record",
                    "A skipped record requires an approved skip_record deviation.",
                    record_id=record["id"],
                )
            action = deviation["proposal"]["action"]
            if (
                record["deviation_id"] != deviation["deviation_id"]
                or record["reason"] != action["reason"]
            ):
                _fail(
                    "attempt_skipped_record_mismatch",
                    "Skipped record evidence must match its approved deviation.",
                    record_id=record["id"],
                )
            recorded.add(record["id"])
        elif deviation is not None:
            _fail(
                "attempt_skip_record_result_mismatch",
                "A record with an approved skip_record deviation must be recorded as skipped.",
                record_id=record["id"],
            )
    if status == "completed":
        missing = sorted(set(approved_skips) - recorded)
        if missing:
            _fail(
                "attempt_missing_skipped_records",
                "A completed Attempt must record every approved skip_record deviation.",
                record_ids=missing,
            )


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
    schema = contract.get("schema") if isinstance(contract, dict) else None
    if schema != "work-attempt/v1":
        _fail("attempt_invalid_schema", "The Attempt schema is invalid.")
    contract = _attempt_model(contract)
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
        "task_collection_sha256",
        "task_index_sha256",
        "task_item_sha256",
        "task_instructions_sha256",
        "execute_instructions_sha256",
        "hierarchy_selection_sha256",
        "execute_skill_selection_sha256",
        "authorization_sha256",
    ):
        _sha256(contract[field], location=field)
    authorization = AttemptAuthorizationContract.model_validate(contract["authorization"]).to_canonical_dict()
    if _authorization_sha256(authorization) != contract["authorization_sha256"]:
        _fail("attempt_authorization_fingerprint_mismatch", "The authorization manifest does not match its fingerprint.")
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

    execution_deviations: list[dict[str, Any]] = []
    if "execution_deviations" in contract:
        raw_deviations = contract["execution_deviations"]
        if not isinstance(raw_deviations, list) or not raw_deviations:
            _fail(
                "attempt_invalid_execution_deviations",
                "execution_deviations must be a non-empty array when present.",
            )
        execution_deviations = [
            ExecutionDeviationContract.model_validate(item).to_canonical_dict()
            for item in raw_deviations
        ]
        expected_ids = [
            f"DEVIATION-{index:03d}"
            for index in range(1, len(execution_deviations) + 1)
        ]
        if [item["deviation_id"] for item in execution_deviations] != expected_ids:
            _fail(
                "attempt_invalid_execution_deviation_sequence",
                "execution_deviations must use contiguous DEVIATION-nnn IDs in stored order.",
            )

    records, operation_outcomes = _validate_records(
        contract["records"], carried_retries=carried_retries
    )
    _validate_skip_records(records, execution_deviations, status=status)
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

    closing_fields = {"final_type", "reason", "closing_authorization_evidence", "ended_at"} & set(contract)
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
            unexpected = {"final_type", "reason", "closing_authorization_evidence"} & set(contract)
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
            missing = {"final_type", "reason", "closing_authorization_evidence"} - set(contract)
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
            _nonempty(
                contract["closing_authorization_evidence"],
                location="closing_authorization_evidence",
            )

    canonical_values: dict[str, Any] = dict(contract)
    canonical_values["authorization"] = authorization
    canonical_values["records"] = records
    if carried_records:
        canonical_values["carried_records"] = carried_records
    if modified_files:
        canonical_values["modified_files"] = modified_files
    if execution_deviations:
        canonical_values["execution_deviations"] = execution_deviations
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
    return AttemptValidationContract(
        schema="work-attempt-validation/v1",
        attempt_id=canonical["attempt_id"],
        task_spec_id=canonical["task_spec_id"],
        task_id=canonical["task_id"],
        status=canonical["status"],
        record_count=len(canonical["records"]),
        result="valid",
    ).to_canonical_dict()


def render_attempt_contract(
    contract: dict[str, Any], *, project_root: Path
) -> bytes:
    canonical = canonicalize_attempt_contract(contract, project_root=project_root)
    return render_json_contract(canonical)


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
    normalized_path = normalize_relative_path(raw_attempt_path, field="attempt_path")
    literal_path = Path(normalized_path)
    if literal_path.name != "attempt.json":
        _fail(
            "attempt_filename_mismatch",
            "Attempt files must use <TASK-ID>/<ATTEMPT-ID>/attempt.json; "
            "legacy flat paths are unsupported.",
        )
    validate_execution_task_layout(
        project_root, literal_path.parent.parent.as_posix()
    )
    _, attempt_path = resolve_project_relative_path(
        project_root, normalized_path, field="attempt_path"
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
    contract = parse_json_contract(raw, source=normalized_path)
    canonical = canonicalize_attempt_contract(contract, project_root=project_root)
    if attempt_path.name != "attempt.json":
        _fail("attempt_filename_mismatch", "The resolved Attempt filename must be attempt.json.")
    if (
        literal_path.parent.name != canonical["attempt_id"]
        or attempt_path.parent.name != canonical["attempt_id"]
    ):
        _fail(
            "attempt_parent_attempt_mismatch",
            "The Attempt directory does not match attempt_id.",
        )
    if (
        literal_path.parent.parent.name != canonical["task_id"]
        or attempt_path.parent.parent.name != canonical["task_id"]
    ):
        _fail(
            "attempt_parent_task_mismatch",
            "The Attempt grandparent directory does not match task_id.",
        )
    require_canonical_json_contract(
        raw,
        contract=canonical,
        source=normalized_path,
    )
    result = validate_attempt_contract(canonical, project_root=project_root)
    result["path"] = normalized_path
    return result


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

from ...technical.infrastructure.text_codec import canonical_sha256
from ...models.execution.index import (
    ATTEMPT_ID_PATTERN, CORRECTION_ID_PATTERN, RECORD_ID_PATTERN,
    TASK_ID_PATTERN, TASK_INSTRUCTION_AUDIT_ID_PATTERN, TASK_STATUSES,
    ExecutionIndexContract,
)


def _index_nonempty_string(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(ExitCode.CONTRACT, "empty_text_value", "A non-empty string is required.", {"location": location})
    return value


def _index_sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A lowercase SHA-256 value is required.", {"location": location})
    return value


def _index_strict_keys(value: object, *, location: str, required: set[str], optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    allowed = required | (optional or set())
    missing, unknown = sorted(required - set(value)), sorted(set(value) - allowed)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    return value

TOP_FIELD_ORDER = (
    "schema",
    "requirement_id",
    "title",
    "task_spec_id",
    "task_collection_sha256",
    "task_index_sha256",
    "task_instructions_sha256",
    "hierarchy_selection_sha256",
    "skill_selection_sha256",
    "instruction_selection_manifest",
    "latest_task_instruction_audit",
    "lock",
    "overall_status",
    "tasks",
)


def _ordered_object(value: object, order: tuple[str, ...]) -> object:
    if not isinstance(value, dict):
        return value
    result = {key: value[key] for key in order if key in value}
    for key in sorted(set(value) - set(order)):
        result[key] = value[key]
    return result


def order_execution_index(contract: dict[str, Any]) -> dict[str, Any]:
    ordered = _ordered_object(contract, TOP_FIELD_ORDER)
    assert isinstance(ordered, dict)
    if "lock" in ordered:
        ordered["lock"] = _ordered_object(
            ordered["lock"],
            (
                "kind",
                "record",
                "task_id",
                "attempt_id",
                "correction_id",
                "record_id",
                "command_correction",
                "execute_instructions_sha256",
                "invalidates_completion",
                "affected_task_ids",
            ),
        )
        if isinstance(ordered["lock"], dict) and "command_correction" in ordered["lock"]:
            ordered["lock"]["command_correction"] = canonicalize_command_correction(
                ordered["lock"]["command_correction"],
                location="lock.command_correction",
            )
    if isinstance(ordered.get("tasks"), list):
        tasks: list[object] = []
        for raw_task in ordered["tasks"]:
            task = _ordered_object(
                raw_task,
                (
                    "id",
                    "status",
                    "skill_id",
                    "task_item_sha256",
                    "instructions_sha256",
                    "latest_attempt",
                    "latest_correction",
                    "status_reason",
                ),
            )
            if isinstance(task, dict) and "status_reason" in task:
                task["status_reason"] = _ordered_object(
                    task["status_reason"], ("kind", "ref")
                )
            tasks.append(task)
        ordered["tasks"] = tasks
    return ordered


def _execution_index_model(contract: object) -> dict[str, Any]:
    try:
        return ExecutionIndexContract.model_validate(contract).to_canonical_dict()
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        missing = sorted(str(issue["loc"][-1]) for issue in issues if issue["type"] == "missing")
        unknown = sorted(str(issue["loc"][-1]) for issue in issues if issue["type"] == "extra_forbidden")
        if missing or unknown:
            raise WorkError(
                ExitCode.CONTRACT, "invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                {"location": "execution_index", "missing": missing, "unknown": unknown},
            ) from error
        raise


def derive_overall_status(statuses: list[str]) -> str:
    active = [status for status in statuses if status != "cancelled"]
    if not active:
        return "cancelled"
    if all(status == "completed" for status in active):
        return "completed"
    if "in_progress" in active:
        return "in_progress"
    unfinished = [status for status in active if status != "completed"]
    if unfinished and all(status == "blocked" for status in unfinished):
        return "blocked"
    return "pending"


def render_execution_index(contract: dict[str, Any]) -> bytes:
    ordered = order_execution_index(contract)
    return render_json_contract(ordered)


def build_initial_execution_index(
    task_contract: dict[str, Any],
    task_validation: dict[str, object],
) -> dict[str, Any]:
    task_instructions = task_validation["task_instructions_sha256"]
    assert isinstance(task_instructions, dict)
    task_skill_ids = task_validation["task_skill_ids"]
    assert isinstance(task_skill_ids, dict)
    result = {
        "schema": "work-execution-index/v1",
        "requirement_id": task_contract["requirement_id"],
        "title": "Execution",
        "task_spec_id": task_contract["spec_id"],
        "task_instructions_sha256": task_validation["instructions_sha256"],
        "hierarchy_selection_sha256": task_validation[
            "hierarchy_selection_sha256"
        ],
        "skill_selection_sha256": task_validation["skill_selection_sha256"],
        "overall_status": "pending",
        "tasks": [
            {
                "id": task["id"],
                "status": "pending",
                "skill_id": task_skill_ids[task["id"]],
                "task_item_sha256": task_validation["task_item_sha256"][task["id"]],
                "instructions_sha256": task_instructions[task["id"]],
            }
            for task in task_contract["tasks"]
        ],
    }
    result["task_collection_sha256"] = task_validation["task_collection_sha256"]
    result["task_index_sha256"] = task_validation["task_index_sha256"]
    return order_execution_index(result)


def validate_execution_index(
    raw: bytes,
    *,
    source: str,
    expected: dict[str, Any] | None = None,
) -> dict[str, object]:
    contract = parse_json_contract(raw, source=source)
    schema = contract.get("schema") if isinstance(contract, dict) else None
    if schema == "work-execution-index/v1":
        fingerprint_fields = {"task_collection_sha256", "task_index_sha256"}
        fingerprint_optional: set[str] = set()
    else:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_execution_index_schema",
            "Invalid execution index schema.",
        )
    index = _execution_index_model(contract)
    title = _index_nonempty_string(index["title"], location="title")
    if "\n" in title or "\r" in title:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_execution_index_title",
            "The execution index title must fit on one line.",
        )
    _index_nonempty_string(index["requirement_id"], location="requirement_id")
    if not re.fullmatch(r"TASK-SPEC-\d{3}", str(index["task_spec_id"])):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_task_spec_id",
            "The execution index TASK spec ID is invalid.",
        )
    for field in sorted(fingerprint_fields | (fingerprint_optional & set(index))):
        _index_sha256(index[field], location=field)
    _index_sha256(
        index["task_instructions_sha256"],
        location="task_instructions_sha256",
    )
    _index_sha256(
        index["hierarchy_selection_sha256"],
        location="hierarchy_selection_sha256",
    )
    _index_sha256(
        index["skill_selection_sha256"],
        location="skill_selection_sha256",
    )
    if "latest_task_instruction_audit" in index and not (
        TASK_INSTRUCTION_AUDIT_ID_PATTERN.fullmatch(
        _index_nonempty_string(
                index["latest_task_instruction_audit"],
                location="latest_task_instruction_audit",
            )
        )
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_task_instruction_audit_id",
            "The latest Task instruction audit ID is invalid.",
        )
    if "lock" in index:
        lock = index["lock"]
        if isinstance(lock, dict) and lock.get("kind") == "spec_update":
            lock = _index_strict_keys(
                lock,
                location="lock",
                required={"kind", "record"},
            )
            if not re.fullmatch(r"SPEC-UPDATE-\d{3}", str(lock["record"])):
                raise WorkError(ExitCode.CONTRACT, "invalid_spec_lock", "Invalid spec lock record.")
        elif isinstance(lock, dict) and lock.get("kind") == "execution":
            lock = _index_strict_keys(
                lock,
                location="lock",
                required={
                    "kind",
                    "task_id",
                    "attempt_id",
                    "execute_instructions_sha256",
                },
                optional={"record_id", "command_correction"},
            )
            if not TASK_ID_PATTERN.fullmatch(str(lock["task_id"])) or not ATTEMPT_ID_PATTERN.fullmatch(str(lock["attempt_id"])):
                raise WorkError(ExitCode.CONTRACT, "invalid_execution_lock", "Invalid execution lock IDs.")
            _index_sha256(
                lock["execute_instructions_sha256"],
                location="lock.execute_instructions_sha256",
            )
            if "record_id" in lock and not RECORD_ID_PATTERN.fullmatch(
                str(lock["record_id"])
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_execution_lock_record_id",
                    "Invalid execution lock record ID.",
                )
            if "command_correction" in lock:
                if not str(lock.get("record_id", "")).startswith("CMD-"):
                    raise WorkError(
                        ExitCode.CONTRACT,
                        "invalid_execution_lock_command_correction",
                        "A command correction requires a reserved CMD record.",
                    )
                canonicalize_command_correction(
                    lock["command_correction"],
                    location="lock.command_correction",
                )
        elif isinstance(lock, dict) and lock.get("kind") == "correction":
            lock = _index_strict_keys(
                lock,
                location="lock",
                required={
                    "kind",
                    "task_id",
                    "attempt_id",
                    "correction_id",
                    "execute_instructions_sha256",
                    "invalidates_completion",
                    "affected_task_ids",
                },
            )
            if (
                not TASK_ID_PATTERN.fullmatch(str(lock["task_id"]))
                or not ATTEMPT_ID_PATTERN.fullmatch(str(lock["attempt_id"]))
                or not CORRECTION_ID_PATTERN.fullmatch(str(lock["correction_id"]))
                or not str(lock["correction_id"]).startswith(
                    f"{lock['attempt_id']}-CORRECTION-"
                )
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_correction_lock",
                    "Invalid Correction lock IDs.",
                )
            _index_sha256(
                lock["execute_instructions_sha256"],
                location="lock.execute_instructions_sha256",
            )
            if not isinstance(lock["invalidates_completion"], bool):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_correction_lock_invalidation",
                    "A Correction lock invalidation flag must be boolean.",
                )
            affected = lock["affected_task_ids"]
            if (
                not isinstance(affected, list)
                or any(
                    not isinstance(item, str)
                    or not TASK_ID_PATTERN.fullmatch(item)
                    for item in affected
                )
                or affected != sorted(set(affected))
                or (not lock["invalidates_completion"] and affected)
                or (lock["invalidates_completion"] and lock["task_id"] not in affected)
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_correction_lock_affected_tasks",
                    "A Correction lock affected TASK list is invalid.",
                )
        else:
            raise WorkError(ExitCode.CONTRACT, "invalid_lock_kind", "Invalid execution index lock kind.")

    raw_tasks = index["tasks"]
    if not isinstance(raw_tasks, list) or not raw_tasks:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_item_array",
            "The execution index tasks array must be non-empty.",
        )
    tasks: list[dict[str, Any]] = []
    previous = 0
    statuses: list[str] = []
    for position, raw_task in enumerate(raw_tasks):
        task_fingerprint_fields = {"task_item_sha256"}
        task = _index_strict_keys(
            raw_task,
            location=f"tasks[{position}]",
            required={
                "id",
                "status",
                "skill_id",
                "instructions_sha256",
                *task_fingerprint_fields,
            },
            optional={"latest_attempt", "latest_correction", "status_reason"},
        )
        match = re.fullmatch(r"TASK-(\d{3})", str(task["id"]))
        if not match or int(match.group(1)) <= previous:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_or_unsorted_id",
                "Execution index TASK IDs must be ascending.",
            )
        previous = int(match.group(1))
        status = task["status"]
        if status not in TASK_STATUSES:
            raise WorkError(ExitCode.CONTRACT, "invalid_task_status", "Invalid TASK status.")
        if task["skill_id"] is not None:
            _index_nonempty_string(task["skill_id"], location=f"{task['id']}.skill_id")
        _index_sha256(
            task["instructions_sha256"],
            location=f"{task['id']}.instructions_sha256",
        )
        if "task_item_sha256" in task:
            _index_sha256(
                task["task_item_sha256"],
                location=f"{task['id']}.task_item_sha256",
            )
        if "latest_attempt" in task and not ATTEMPT_ID_PATTERN.fullmatch(
            _index_nonempty_string(task["latest_attempt"], location=f"{task['id']}.latest_attempt")
        ):
            raise WorkError(ExitCode.CONTRACT, "invalid_attempt_id", "Invalid latest Attempt ID.")
        if "latest_correction" in task and not CORRECTION_ID_PATTERN.fullmatch(
            _index_nonempty_string(
                task["latest_correction"], location=f"{task['id']}.latest_correction"
            )
        ):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_correction_id",
                "Invalid latest Correction ID.",
            )
        if "status_reason" in task:
            reason = _index_strict_keys(
                task["status_reason"],
                location=f"{task['id']}.status_reason",
                required={"kind", "ref"},
            )
            if reason["kind"] not in {
                "attempt",
                "correction",
                "task_change",
                "instruction_audit",
            }:
                raise WorkError(ExitCode.CONTRACT, "invalid_status_reason", "Invalid status reason kind.")
            _index_nonempty_string(reason["ref"], location=f"{task['id']}.status_reason.ref")
            if reason["kind"] == "correction" and not CORRECTION_ID_PATTERN.fullmatch(
                str(reason["ref"])
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_status_reason",
                    "A Correction status reason must reference a Correction ID.",
                )
            if reason["kind"] == "instruction_audit" and not (
                TASK_INSTRUCTION_AUDIT_ID_PATTERN.fullmatch(str(reason["ref"]))
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_status_reason",
                    "An instruction audit status reason must reference a Task instruction audit ID.",
                )
        if status in {"in_progress", "completed"} and "latest_attempt" not in task:
            raise WorkError(ExitCode.CONTRACT, "latest_attempt_required", "This TASK status requires latest_attempt.")
        if status in {"pending_retry", "blocked", "cancelled"} and "status_reason" not in task:
            raise WorkError(ExitCode.CONTRACT, "status_reason_required", "This TASK status requires status_reason.")
        if status == "pending" and ({"latest_attempt", "latest_correction", "status_reason"} & set(task)):
            raise WorkError(ExitCode.CONTRACT, "invalid_pending_metadata", "An initial pending TASK cannot have attempt metadata.")
        statuses.append(status)
        tasks.append(task)
    if index["overall_status"] != derive_overall_status(statuses):
        raise WorkError(
            ExitCode.CONTRACT,
            "overall_status_mismatch",
            "overall_status does not match the TASK statuses.",
        )
    if expected is not None and index != expected:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execution_index_mismatch",
            "The execution index does not match the expected TASK state.",
        )
    ordered = order_execution_index(index)
    require_canonical_json_contract(
        raw,
        contract=ordered,
        source=source,
    )
    return {
        "schema": "work-execution-index-validation/v1",
        "requirement_id": index["requirement_id"],
        "task_spec_id": index["task_spec_id"],
        "overall_status": index["overall_status"],
        "index_sha256": canonical_sha256(raw, source=source),
        "task_count": len(tasks),
    }
