from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import validate_artifact_paths


HANDOFF_MARKER = "WORK-HANDOFF"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TASK_SPEC_PATTERN = re.compile(r"^TASK-SPEC-\d{3}$")
TASK_PATTERN = re.compile(r"^TASK-\d{3}$")
ATTEMPT_PATTERN = re.compile(r"^ATTEMPT-\d{3}$")
AFFECTED_ID_PATTERN = re.compile(r"^[A-Z][A-Z0-9-]*-\d{3}$")
DIRECTION_STAGES = {
    "plan_to_task": ("plan", "task"),
    "task_to_execute": ("task", "execute"),
    "execute_to_task": ("execute", "task"),
    "task_to_plan": ("task", "plan"),
    "execute_to_plan": ("execute", "plan"),
}
RETURN_DIRECTIONS = {"execute_to_task", "task_to_plan", "execute_to_plan"}
COMMON_FIELDS = {
    "schema",
    "marker",
    "direction",
    "requirement_id",
    "artifacts",
    "source",
    "target",
    "summary",
}
RETURN_FIELDS = {
    "confirmed_approach",
    "requested_changes",
    "preserve",
    "affected_ids",
    "validation_requirements",
}


def _strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
    optional: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "expected_object",
            "A JSON object is required.",
            {"location": location},
        )
    allowed = required | (optional or set())
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - allowed)
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": location, "missing": missing, "unknown": unknown},
        )
    return value


def _nonempty_string(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(
            ExitCode.CONTRACT,
            "empty_text_value",
            "A non-empty string is required.",
            {"location": location},
        )
    return value


def _string_array(value: object, *, location: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A non-empty string array is required.",
            {"location": location},
        )
    result = [
        _nonempty_string(item, location=f"{location}[]") for item in value
    ]
    if len(result) != len(set(result)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_array_value",
            "Array values must be unique.",
            {"location": location},
        )
    return result


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A lowercase SHA-256 value is required.",
            {"location": location},
        )
    return value


def _identifier(value: object, *, location: str, pattern: re.Pattern[str]) -> str:
    identifier = _nonempty_string(value, location=location)
    if not pattern.fullmatch(identifier):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_handoff_identifier",
            "A Handoff identifier has an invalid format.",
            {"location": location, "value": identifier},
        )
    return identifier


def _skill_id(value: object, *, location: str) -> str | None:
    if value is None:
        return None
    return _nonempty_string(value, location=location)


def _validate_execution_context(value: object) -> None:
    context = _strict_object(
        value,
        location="source.execution_context",
        required={"attempt", "phase", "issue_type", "reason"},
    )
    attempt = _strict_object(
        context["attempt"],
        location="source.execution_context.attempt",
        required={"status"},
        optional={"id"},
    )
    status = attempt["status"]
    if status not in {"not_created", "stopped", "blocked"}:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_attempt_status",
            "The Handoff Attempt status is invalid.",
        )
    if status == "not_created":
        if "id" in attempt:
            raise WorkError(
                ExitCode.CONTRACT,
                "unexpected_attempt_id",
                "A not-created Attempt cannot have an ID.",
            )
    elif "id" not in attempt:
        raise WorkError(
            ExitCode.CONTRACT,
            "missing_attempt_id",
            "A stopped or blocked Attempt must have an ID.",
        )
    else:
        _identifier(
            attempt["id"],
            location="source.execution_context.attempt.id",
            pattern=ATTEMPT_PATTERN,
        )
    if context["phase"] not in {"preflight", "execution"}:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_execution_phase",
            "The Handoff execution phase is invalid.",
        )
    if context["issue_type"] != "specification_defect":
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_issue_type",
            "An Execute return Handoff must describe a specification defect.",
        )
    _nonempty_string(context["reason"], location="source.execution_context.reason")


def _validate_source(value: object, direction: str) -> None:
    if direction == "plan_to_task":
        source = _strict_object(
            value,
            location="source",
            required={"stage", "plan_sha256", "skill_selection_sha256"},
        )
        _sha256(source["plan_sha256"], location="source.plan_sha256")
        _sha256(source["skill_selection_sha256"], location="source.skill_selection_sha256")
    elif direction == "task_to_plan":
        source = _strict_object(
            value,
            location="source",
            required={"stage", "plan_sha256", "task_spec_id", "skill_selection_sha256"},
            optional={"task_id", "skill_id"},
        )
        _sha256(source["plan_sha256"], location="source.plan_sha256")
        _sha256(source["skill_selection_sha256"], location="source.skill_selection_sha256")
        _identifier(
            source["task_spec_id"],
            location="source.task_spec_id",
            pattern=TASK_SPEC_PATTERN,
        )
        if "task_id" in source:
            _identifier(
                source["task_id"], location="source.task_id", pattern=TASK_PATTERN
            )
        if "skill_id" in source:
            _skill_id(source["skill_id"], location="source.skill_id")
    else:
        required = {
            "stage",
            "task_spec_id",
            "task_id",
            "task_sha256",
            "task_instructions_sha256",
            "skill_id",
        }
        if direction.startswith("execute_to_"):
            required.update({"execution_context", "execute_skill_selection_sha256"})
        else:
            required.add("skill_selection_sha256")
        source = _strict_object(value, location="source", required=required)
        _identifier(
            source["task_spec_id"],
            location="source.task_spec_id",
            pattern=TASK_SPEC_PATTERN,
        )
        _identifier(source["task_id"], location="source.task_id", pattern=TASK_PATTERN)
        _sha256(source["task_sha256"], location="source.task_sha256")
        _sha256(
            source["task_instructions_sha256"],
            location="source.task_instructions_sha256",
        )
        _skill_id(source["skill_id"], location="source.skill_id")
        if direction.startswith("execute_to_"):
            _sha256(
                source["execute_skill_selection_sha256"],
                location="source.execute_skill_selection_sha256",
            )
            _validate_execution_context(source["execution_context"])
        else:
            _sha256(source["skill_selection_sha256"], location="source.skill_selection_sha256")

    expected_stage = DIRECTION_STAGES[direction][0]
    if source["stage"] != expected_stage:
        raise WorkError(
            ExitCode.CONTRACT,
            "handoff_source_stage_mismatch",
            "The Handoff source stage does not match its direction.",
            {"expected": expected_stage, "actual": source["stage"]},
        )


def _validate_target(value: object, direction: str) -> None:
    target = _strict_object(
        value,
        location="target",
        required={"stage"},
    )
    expected_stage = DIRECTION_STAGES[direction][1]
    if target["stage"] != expected_stage:
        raise WorkError(
            ExitCode.CONTRACT,
            "handoff_target_stage_mismatch",
            "The Handoff target stage does not match its direction.",
            {"expected": expected_stage, "actual": target["stage"]},
        )


def validate_handoff_contract(
    contract: dict[str, Any], *, project_root: Path
) -> dict[str, object]:
    direction = contract.get("direction")
    if direction not in DIRECTION_STAGES:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_handoff_direction",
            "The Handoff direction is invalid.",
            {"direction": direction},
        )
    required = set(COMMON_FIELDS)
    if direction == "plan_to_task":
        required.add("affected_ids")
    elif direction in RETURN_DIRECTIONS:
        required.update(RETURN_FIELDS)
    _strict_object(contract, location="handoff", required=required)

    if contract["schema"] != "work-handoff/v1":
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_handoff_schema",
            "The Handoff schema is invalid.",
        )
    if contract["marker"] != HANDOFF_MARKER:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_handoff_marker",
            "The Handoff marker is invalid.",
        )

    requirement_id = _nonempty_string(
        contract["requirement_id"], location="requirement_id"
    )
    normalized_artifacts = validate_artifact_paths(
        project_root,
        requirement_id,
        contract["artifacts"],
        actual_plan_path=contract["artifacts"].get("plan")
        if isinstance(contract["artifacts"], dict)
        else "",
    )
    if contract["artifacts"] != normalized_artifacts:
        raise WorkError(
            ExitCode.CONTRACT,
            "noncanonical_artifact_paths",
            "Handoff artifact paths must use their normalized project-relative form.",
        )

    _validate_source(contract["source"], direction)
    _validate_target(contract["target"], direction)
    _nonempty_string(contract["summary"], location="summary")

    if direction == "plan_to_task":
        affected_ids = _string_array(
            contract["affected_ids"], location="affected_ids"
        )
    elif direction in RETURN_DIRECTIONS:
        _nonempty_string(
            contract["confirmed_approach"], location="confirmed_approach"
        )
        _string_array(contract["requested_changes"], location="requested_changes")
        _string_array(contract["preserve"], location="preserve")
        affected_ids = _string_array(
            contract["affected_ids"], location="affected_ids"
        )
        _string_array(
            contract["validation_requirements"],
            location="validation_requirements",
        )
    else:
        affected_ids = []

    for identifier in affected_ids:
        _identifier(
            identifier, location="affected_ids[]", pattern=AFFECTED_ID_PATTERN
        )

    return {
        "schema": "work-handoff-validation/v1",
        "marker": HANDOFF_MARKER,
        "direction": direction,
        "requirement_id": requirement_id,
        "source_stage": DIRECTION_STAGES[direction][0],
        "target_stage": DIRECTION_STAGES[direction][1],
        "status": "valid",
    }


def validate_handoff_json_contract(
    raw: bytes, *, source: str, project_root: Path
) -> dict[str, object]:
    contract = parse_json_contract(raw, source=source)
    return validate_handoff_contract(contract, project_root=project_root)


def render_handoff_json_contract(
    raw: bytes, *, source: str, project_root: Path
) -> dict[str, Any]:
    contract = parse_json_contract(raw, source=source)
    validate_handoff_contract(contract, project_root=project_root)
    return contract
