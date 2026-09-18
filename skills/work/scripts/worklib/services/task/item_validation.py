from __future__ import annotations

import re
from typing import Any

from ...protocol import TASK_ID_PATTERN as TASK_ID_PATTERN_TEXT
from ...models.common.errors import ExitCode, WorkError
from ...technical.infrastructure.text_codec import canonical_sha256
from ...technical.infrastructure.json_contract import (
    parse_json_contract,
    render_json_contract,
    require_canonical_json_contract,
)
from ...models.task_collection import TaskItemContract, TaskItemValidationContract
from ...models.common.validation import ContractValuePolicy


nonempty_string = ContractValuePolicy.nonempty_string
strict_keys = ContractValuePolicy.strict_keys


TASK_ID_PATTERN = re.compile(TASK_ID_PATTERN_TEXT)


def _string_array(value: object, *, location: str, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A string array with the required cardinality is required.",
            {"location": location},
        )
    result = [nonempty_string(item, location=f"{location}[]") for item in value]
    if len(result) != len(set(result)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_array_value", "Array values must be unique.", {"location": location})
    return result


def _items(
    task: dict[str, Any],
    key: str,
    prefix: str,
    *,
    required: set[str],
    optional: set[str] | None = None,
) -> list[dict[str, Any]]:
    raw_items = task.get(key)
    if not isinstance(raw_items, list) or not raw_items:
        raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "A present TASK item array must be non-empty.", {"location": key})
    result: list[dict[str, Any]] = []
    previous = 0
    for index, raw_item in enumerate(raw_items):
        item = strict_keys(raw_item, location=f"{key}[{index}]", required={"id", *required}, optional=optional)
        item_id = nonempty_string(item["id"], location=f"{key}[{index}].id")
        match = re.fullmatch(rf"{re.escape(prefix)}-(\d{{3}})", item_id)
        if not match or int(match.group(1)) <= previous:
            raise WorkError(ExitCode.CONTRACT, "invalid_or_unsorted_id", "TASK item IDs must use the expected prefix and ascending order.")
        previous = int(match.group(1))
        result.append(item)
    return result


def render_task_item_contract(contract: dict[str, Any], *, ordered_contract: dict[str, Any]) -> bytes:
    return render_json_contract(ordered_contract)


def validate_task_item_contract(raw: bytes, *, source: str, expected_task_id: str, ordered_contract: dict[str, Any], required_fields: set[str], optional_fields: set[str]) -> dict[str, object]:
    untyped = parse_json_contract(raw, source=source)
    missing = sorted(
        field for field in {"schema", *required_fields}
        if field not in untyped or (field != "skill_id" and untyped[field] is None)
    )
    unknown = sorted(set(untyped) - {"schema", *required_fields, *optional_fields})
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": "contract", "missing": missing, "unknown": unknown},
        )
    model = TaskItemContract.parse_json_bytes(raw, source=source)
    contract = model.to_canonical_dict()
    strict_keys(
        contract,
        location="task_item",
        required={"schema", *required_fields},
        optional=optional_fields,
    )
    task_id = nonempty_string(contract["id"], location="id")
    if not TASK_ID_PATTERN.fullmatch(task_id) or task_id != expected_task_id:
        raise WorkError(ExitCode.CONTRACT, "task_item_identity_mismatch", "The TASK item ID does not match its reference.")
    for field in ("title", "goal"):
        nonempty_string(contract[field], location=field)
    if contract["skill_id"] is not None:
        nonempty_string(contract["skill_id"], location="skill_id")

    traceability = strict_keys(
        contract["traceability"],
        location="traceability",
        required={"goal_ids", "deliverable_ids", "acceptance_ids"},
        optional={"milestone_ids"},
    )
    prefixes = {
        "goal_ids": "GOAL",
        "deliverable_ids": "DELIVERABLE",
        "acceptance_ids": "ACCEPTANCE",
        "milestone_ids": "MILESTONE",
    }
    for field, prefix in prefixes.items():
        if field not in traceability:
            continue
        values = _string_array(traceability[field], location=f"traceability.{field}")
        if any(not re.fullmatch(rf"{prefix}-\d{{3}}", value) for value in values):
            raise WorkError(ExitCode.CONTRACT, "invalid_reference", "A traceability ID has the wrong type.")

    dependencies = _string_array(contract.get("dependencies", []), location="dependencies", allow_empty=True)
    if any(not TASK_ID_PATTERN.fullmatch(value) or value == task_id for value in dependencies):
        raise WorkError(ExitCode.CONTRACT, "invalid_task_dependency", "A TASK dependency is invalid.")

    local_ids: set[str] = set()
    if "inputs" in contract:
        for item in _items(contract, "inputs", "INPUT", required={"kind", "source", "precondition"}):
            if item["kind"] not in {"task_output", "project_state", "user_provided", "external"}:
                raise WorkError(ExitCode.CONTRACT, "invalid_input_kind", "Invalid TASK input kind.")
            nonempty_string(item["source"], location=f"{item['id']}.source")
            nonempty_string(item["precondition"], location=f"{item['id']}.precondition")
            local_ids.add(item["id"])
    if "decisions" in contract:
        for item in _items(contract, "decisions", "TASK-DECISION", required={"statement", "rationale"}):
            nonempty_string(item["statement"], location=f"{item['id']}.statement")
            nonempty_string(item["rationale"], location=f"{item['id']}.rationale")
            local_ids.add(item["id"])
    file_ids: set[str] = set()
    if "files" in contract:
        for item in _items(contract, "files", "FILE", required={"action"}, optional={"path", "source", "destination"}):
            expected = {
                "create": {"id", "action", "path"},
                "modify": {"id", "action", "path"},
                "move": {"id", "action", "source", "destination"},
            }
            if item["action"] not in expected or set(item) != expected[item["action"]]:
                raise WorkError(ExitCode.CONTRACT, "invalid_file_action", "Invalid fields for TASK file action.")
            for field in ("path", "source", "destination"):
                if field in item:
                    nonempty_string(item[field], location=f"{item['id']}.{field}")
            file_ids.add(item["id"])
            local_ids.add(item["id"])
    if "risks" in contract:
        for item in _items(contract, "risks", "RISK", required={"condition", "impact", "mitigation"}):
            for field in ("condition", "impact", "mitigation"):
                nonempty_string(item[field], location=f"{item['id']}.{field}")
            local_ids.add(item["id"])

    command_ids: set[str] = set()
    if "commands" in contract:
        for item in _items(contract, "commands", "CMD", required={"mode"}, optional={"argv", "script", "execution"}):
            allowed = {"id", "mode", "argv"} if item["mode"] == "argv" else ({"id", "mode", "script"} if item["mode"] == "shell" else set())
            if "execution" in item:
                allowed |= {"execution"}
            if not allowed or set(item) != allowed:
                raise WorkError(ExitCode.CONTRACT, "invalid_command_mode", "Invalid command mode or fields.")
            if item["mode"] == "argv":
                _string_array(item["argv"], location=f"{item['id']}.argv")
            else:
                nonempty_string(item["script"], location=f"{item['id']}.script")
            if "execution" in item:
                execution = strict_keys(item["execution"], location=f"{item['id']}.execution", required={"working_directory", "os", "shell"})
                for field in ("working_directory", "os", "shell"):
                    nonempty_string(execution[field], location=f"{item['id']}.execution.{field}")
            command_ids.add(item["id"])
            local_ids.add(item["id"])

    validation_ids: set[str] = set()
    for item in _items(
        contract,
        "validations",
        "VAL",
        required={"kind"},
        optional={"command_ids", "pass_condition", "confirmer", "criteria", "acceptance_ids"},
    ):
        common = {"id", "kind"} | ({"acceptance_ids"} if "acceptance_ids" in item else set())
        if item["kind"] == "automated" and set(item) == common | {"command_ids", "pass_condition"}:
            referenced = _string_array(item["command_ids"], location=f"{item['id']}.command_ids")
            if any(command_id not in command_ids for command_id in referenced):
                raise WorkError(ExitCode.CONTRACT, "invalid_reference", "Automated validation references an unknown command.")
            nonempty_string(item["pass_condition"], location=f"{item['id']}.pass_condition")
        elif item["kind"] == "manual" and set(item) == common | {"confirmer", "criteria"}:
            nonempty_string(item["confirmer"], location=f"{item['id']}.confirmer")
            nonempty_string(item["criteria"], location=f"{item['id']}.criteria")
        else:
            raise WorkError(ExitCode.CONTRACT, "invalid_validation_kind", "Invalid validation kind or fields.")
        if "acceptance_ids" in item:
            _string_array(item["acceptance_ids"], location=f"{item['id']}.acceptance_ids")
        validation_ids.add(item["id"])
        local_ids.add(item["id"])

    operation_ids: set[str] = set()
    if "operations" in contract:
        for item in _items(contract, "operations", "OP", required={"kind", "action", "target", "validation_id"}, optional={"command_id"}):
            if item["kind"] not in {"local_state", "external_state"} or item["validation_id"] not in validation_ids:
                raise WorkError(ExitCode.CONTRACT, "invalid_reference", "Invalid TASK operation kind or validation reference.")
            if "command_id" in item and item["command_id"] not in command_ids:
                raise WorkError(ExitCode.CONTRACT, "invalid_reference", "TASK operation references an unknown command.")
            nonempty_string(item["action"], location=f"{item['id']}.action")
            nonempty_string(item["target"], location=f"{item['id']}.target")
            operation_ids.add(item["id"])
            local_ids.add(item["id"])

    referenced: set[str] = set()
    for item in _items(contract, "steps", "STEP", required={"action", "references"}):
        nonempty_string(item["action"], location=f"{item['id']}.action")
        references = _string_array(item["references"], location=f"{item['id']}.references")
        if any(
            reference not in local_ids
            and not re.fullmatch(r"DECISION-\d{3}", reference)
            for reference in references
        ):
            raise WorkError(ExitCode.CONTRACT, "invalid_reference", "STEP references an unknown TASK-local ID.")
        referenced.update(references)
    required_references = file_ids | command_ids | validation_ids | operation_ids
    if not required_references.issubset(referenced):
        raise WorkError(ExitCode.CONTRACT, "unreferenced_task_item", "Every FILE, CMD, OP, and VAL must be referenced by a STEP.")

    require_canonical_json_contract(raw, contract=ordered_contract, source=source)
    return TaskItemValidationContract.model_validate({
        "schema": "work-task-item-validation/v1",
        "task_id": task_id,
        "dependencies": dependencies,
        "task_item_sha256": canonical_sha256(raw, source=source),
    }).to_canonical_dict()
