from __future__ import annotations

import re
from typing import Any

from .attempt import canonicalize_command_correction
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256
from ..foundation.markdown import (
    parse_markdown_json_contract,
    render_markdown_json_contract,
    require_canonical_markdown_json_contract,
)


SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TASK_ID_PATTERN = re.compile(r"^TASK-\d{3}$")
ATTEMPT_ID_PATTERN = re.compile(r"^ATTEMPT-\d{3}$")
CORRECTION_ID_PATTERN = re.compile(
    r"^ATTEMPT-\d{3}-CORRECTION-\d{3}$"
)
TASK_INSTRUCTION_AUDIT_ID_PATTERN = re.compile(
    r"^TASK-INSTRUCTION-AUDIT-\d{3}$"
)
RECORD_ID_PATTERN = re.compile(r"^(?:CMD|OP|VAL)-\d{3}(?:#[1-9]\d*)?$")
TASK_STATUSES = {
    "pending",
    "in_progress",
    "pending_retry",
    "blocked",
    "completed",
    "cancelled",
}
TOP_FIELD_ORDER = (
    "schema",
    "requirement_id",
    "title",
    "task_spec_id",
    "task_sha256",
    "task_instructions_sha256",
    "hierarchy_selection_sha256",
    "skill_selection_sha256",
    "latest_task_instruction_audit",
    "lock",
    "overall_status",
    "tasks",
)


def _strict_keys(
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


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            {"location": location},
        )
    return value


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


def render_execution_index(contract: dict[str, Any]) -> bytes:
    ordered = order_execution_index(contract)
    title = ordered.get("title")
    return render_markdown_json_contract(
        title if isinstance(title, str) else str(title), ordered
    )


def build_initial_execution_index(
    task_contract: dict[str, Any],
    task_validation: dict[str, object],
) -> dict[str, Any]:
    task_instructions = task_validation["task_instructions_sha256"]
    assert isinstance(task_instructions, dict)
    task_skill_ids = task_validation["task_skill_ids"]
    assert isinstance(task_skill_ids, dict)
    return {
        "schema": "work-execution-index/v1",
        "requirement_id": task_contract["requirement_id"],
        "title": "Execution",
        "task_spec_id": task_contract["spec_id"],
        "task_sha256": task_validation["task_sha256"],
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
                "instructions_sha256": task_instructions[task["id"]],
            }
            for task in task_contract["tasks"]
        ],
    }


def validate_execution_index(
    raw: bytes,
    *,
    source: str,
    expected: dict[str, Any] | None = None,
) -> dict[str, object]:
    markdown_title, contract = parse_markdown_json_contract(raw, source=source)
    index = _strict_keys(
        contract,
        location="execution_index",
        required={
            "schema",
            "requirement_id",
            "title",
            "task_spec_id",
            "task_sha256",
            "task_instructions_sha256",
            "hierarchy_selection_sha256",
            "skill_selection_sha256",
            "overall_status",
            "tasks",
        },
        optional={"latest_task_instruction_audit", "lock"},
    )
    if index["schema"] != "work-execution-index/v1":
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_execution_index_schema",
            "Invalid execution index schema.",
        )
    title = _nonempty_string(index["title"], location="title")
    if title != markdown_title or "\n" in title or "\r" in title:
        raise WorkError(
            ExitCode.CONTRACT,
            "execution_index_title_mismatch",
            "The execution index H1 and title must match on one line.",
        )
    _nonempty_string(index["requirement_id"], location="requirement_id")
    if not re.fullmatch(r"TASK-SPEC-\d{3}", str(index["task_spec_id"])):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_task_spec_id",
            "The execution index TASK spec ID is invalid.",
        )
    _sha256(index["task_sha256"], location="task_sha256")
    _sha256(
        index["task_instructions_sha256"],
        location="task_instructions_sha256",
    )
    _sha256(
        index["hierarchy_selection_sha256"],
        location="hierarchy_selection_sha256",
    )
    _sha256(
        index["skill_selection_sha256"],
        location="skill_selection_sha256",
    )
    if "latest_task_instruction_audit" in index and not (
        TASK_INSTRUCTION_AUDIT_ID_PATTERN.fullmatch(
        _nonempty_string(
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
            lock = _strict_keys(
                lock,
                location="lock",
                required={"kind", "record"},
            )
            if not re.fullmatch(r"SPEC-UPDATE-\d{3}", str(lock["record"])):
                raise WorkError(ExitCode.CONTRACT, "invalid_spec_lock", "Invalid spec lock record.")
        elif isinstance(lock, dict) and lock.get("kind") == "execution":
            lock = _strict_keys(
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
            _sha256(
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
            lock = _strict_keys(
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
            _sha256(
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
        task = _strict_keys(
            raw_task,
            location=f"tasks[{position}]",
            required={"id", "status", "skill_id", "instructions_sha256"},
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
            _nonempty_string(task["skill_id"], location=f"{task['id']}.skill_id")
        _sha256(
            task["instructions_sha256"],
            location=f"{task['id']}.instructions_sha256",
        )
        if "latest_attempt" in task and not ATTEMPT_ID_PATTERN.fullmatch(
            _nonempty_string(task["latest_attempt"], location=f"{task['id']}.latest_attempt")
        ):
            raise WorkError(ExitCode.CONTRACT, "invalid_attempt_id", "Invalid latest Attempt ID.")
        if "latest_correction" in task and not CORRECTION_ID_PATTERN.fullmatch(
            _nonempty_string(
                task["latest_correction"], location=f"{task['id']}.latest_correction"
            )
        ):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_correction_id",
                "Invalid latest Correction ID.",
            )
        if "status_reason" in task:
            reason = _strict_keys(
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
            _nonempty_string(reason["ref"], location=f"{task['id']}.status_reason.ref")
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
    require_canonical_markdown_json_contract(
        raw,
        title=title,
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
