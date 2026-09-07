from __future__ import annotations

from typing import Any


TOP_FIELD_ORDER = (
    "schema",
    "requirement_id",
    "spec_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "source_plan",
    "instruction_selection",
    "execution_defaults",
    "decisions",
    "tasks",
    "changes",
    "readiness",
)
TASK_FIELD_ORDER = (
    "id",
    "title",
    "skill_id",
    "instruction_selection",
    "traceability",
    "dependencies",
    "inputs",
    "decisions",
    "goal",
    "files",
    "risks",
    "steps",
    "commands",
    "operations",
    "validations",
)


def _ordered_object(value: object, order: tuple[str, ...]) -> object:
    if not isinstance(value, dict):
        return value
    result = {key: value[key] for key in order if key in value}
    for key in sorted(set(value) - set(order)):
        result[key] = value[key]
    return result


def _order_array(value: object, order: tuple[str, ...]) -> object:
    if not isinstance(value, list):
        return value
    return [_ordered_object(item, order) for item in value]


def order_task_contract(contract: dict[str, Any]) -> dict[str, Any]:
    ordered = _ordered_object(contract, TOP_FIELD_ORDER)
    assert isinstance(ordered, dict)
    if "artifacts" in ordered:
        ordered["artifacts"] = _ordered_object(
            ordered["artifacts"], ("plan", "task", "execution")
        )
    if "source_plan" in ordered:
        ordered["source_plan"] = _ordered_object(
            ordered["source_plan"],
            ("canonical_sha256", "hierarchy_selection_sha256"),
        )
    if "instruction_selection" in ordered:
        selection = _ordered_object(
            ordered["instruction_selection"],
            ("sources", "references", "instructions_sha256"),
        )
        if isinstance(selection, dict):
            selection["sources"] = _order_array(
                selection.get("sources"),
                ("kind", "logical_name", "canonical_sha256"),
            )
        ordered["instruction_selection"] = selection
    if "execution_defaults" in ordered:
        ordered["execution_defaults"] = _ordered_object(
            ordered["execution_defaults"], ("working_directory", "os", "shell")
        )
    if "decisions" in ordered:
        ordered["decisions"] = _order_array(
            ordered["decisions"], ("id", "statement", "rationale", "task_ids")
        )
    if isinstance(ordered.get("tasks"), list):
        tasks: list[object] = []
        for raw_task in ordered["tasks"]:
            task = _ordered_object(raw_task, TASK_FIELD_ORDER)
            if not isinstance(task, dict):
                tasks.append(task)
                continue
            if "instruction_selection" in task:
                selection = _ordered_object(
                    task["instruction_selection"],
                    (
                        "selected_paths",
                        "resolved_paths",
                        "sources",
                        "references",
                        "instructions_sha256",
                    ),
                )
                if isinstance(selection, dict):
                    selection["sources"] = _order_array(
                        selection.get("sources"),
                        ("kind", "logical_name", "canonical_sha256"),
                    )
                task["instruction_selection"] = selection
            if "traceability" in task:
                task["traceability"] = _ordered_object(
                    task["traceability"],
                    ("goal_ids", "deliverable_ids", "acceptance_ids", "milestone_ids"),
                )
            if "inputs" in task:
                task["inputs"] = _order_array(
                    task["inputs"], ("id", "kind", "source", "precondition")
                )
            if "decisions" in task:
                task["decisions"] = _order_array(
                    task["decisions"], ("id", "statement", "rationale")
                )
            if "files" in task:
                task["files"] = _order_array(
                    task["files"], ("id", "action", "path", "source", "destination")
                )
            if "risks" in task:
                task["risks"] = _order_array(
                    task["risks"], ("id", "condition", "impact", "mitigation")
                )
            task["steps"] = _order_array(
                task.get("steps"), ("id", "action", "references")
            )
            if "commands" in task:
                task["commands"] = _order_array(
                    task["commands"], ("id", "mode", "argv", "script", "execution")
                )
            if isinstance(task.get("commands"), list):
                for command in task["commands"]:
                    if isinstance(command, dict) and "execution" in command:
                        command["execution"] = _ordered_object(
                            command["execution"], ("working_directory", "os", "shell")
                        )
            if "operations" in task:
                task["operations"] = _order_array(
                    task["operations"],
                    ("id", "kind", "action", "target", "command_id", "validation_id"),
                )
            task["validations"] = _order_array(
                task.get("validations"),
                (
                    "id",
                    "kind",
                    "command_ids",
                    "pass_condition",
                    "confirmer",
                    "criteria",
                    "acceptance_ids",
                ),
            )
            tasks.append(task)
        ordered["tasks"] = tasks
    if "changes" in ordered:
        ordered["changes"] = _order_array(
            ordered["changes"],
            ("id", "spec_id", "date", "reason", "affected_ids", "plan_change_ids", "edits"),
        )
    if isinstance(ordered.get("changes"), list):
        for change in ordered["changes"]:
            if isinstance(change, dict):
                change["edits"] = _order_array(
                    change.get("edits"), ("operation", "path", "before", "after")
                )
    if "readiness" in ordered:
        ordered["readiness"] = _ordered_object(
            ordered["readiness"], ("status", "spec_id")
        )
    return ordered
