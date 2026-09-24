"""Formalize semantic deviation actions against verified TASK records."""
from __future__ import annotations

from typing import Any

from ...models.common.errors import ExitCode, WorkError


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, details or None)


def formalize_semantic_action(action: dict[str, Any], task: dict[str, Any], attempt: dict[str, Any], anchor: str) -> dict[str, Any]:
    kind = action["kind"]
    base = anchor.split("#", 1)[0]
    prefix = base.split("-", 1)[0]

    def position(group: str, value: int) -> str:
        rows = (task.get("traceability", {}).get("acceptance_ids") or []) if group == "acceptance" else (task.get(group) or [])
        if value > len(rows):
            _fail("deviation_semantic_reference", "A semantic position does not exist in the current TASK.", group=group, position=value)
        return rows[value - 1] if group == "acceptance" else rows[value - 1]["id"]

    def positions(group: str, values: list[int]) -> list[str]:
        if not values or len(values) != len(set(values)):
            _fail("deviation_semantic_reference", "Semantic positions must be nonempty and unique.", group=group)
        return [position(group, value) for value in values]

    def next_id(group: str, id_prefix: str) -> str:
        used = {row["id"] for row in task.get(group) or []}
        for deviation in attempt.get("execution_deviations", []):
            prior = deviation["proposal"]["action"]
            if prior["kind"] == ("add_command" if group == "commands" else "add_validation"):
                used.add(prior["command" if group == "commands" else "validation"]["id"])
        number = max((int(value.split("-", 1)[1]) for value in used if value.startswith(id_prefix + "-") and value.split("-", 1)[1].isdigit()), default=0) + 1
        if number > 999:
            _fail("deviation_semantic_id_limit", "The current TASK has no available formal deviation ID.", group=group)
        return f"{id_prefix}-{number:03d}"

    if kind == "replace_command":
        if prefix != "CMD":
            _fail("deviation_action_anchor", "replace_command requires a reserved command record.")
        return {"kind": kind, "record_id": anchor, "replacement": action["replacement"]}
    if kind == "skip_record":
        return {"kind": kind, "record_id": anchor, "reason": action["reason"]}
    if kind == "add_command":
        return {"kind": kind, "after_record_id": anchor, "command": {
            "id": next_id("commands", "CMD"), **action["command"]}}
    if kind == "add_validation":
        validation = dict(action["validation"])
        if "command_positions" in validation:
            validation["command_ids"] = positions("commands", validation.pop("command_positions"))
        if "acceptance_positions" in validation:
            validation["acceptance_ids"] = positions("acceptance", validation.pop("acceptance_positions"))
        return {"kind": kind, "validation": {"id": next_id("validations", "VAL"), **validation}}
    if prefix != "OP":
        _fail("deviation_action_anchor", "adjust_operation requires a reserved operation record.")
    operation = dict(action["operation"])
    operation["validation_id"] = position("validations", operation.pop("validation_position"))
    if "command_position" in operation:
        operation["command_id"] = position("commands", operation.pop("command_position"))
    return {"kind": kind, "operation": {"id": base, **operation}}
