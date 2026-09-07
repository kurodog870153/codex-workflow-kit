from __future__ import annotations

from typing import Any

from .command_correction import canonicalize_command_correction


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
