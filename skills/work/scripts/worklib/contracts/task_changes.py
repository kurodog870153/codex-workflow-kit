from __future__ import annotations

import re
from datetime import date

from ..foundation.errors import ExitCode, WorkError
from .validation import nonempty_string, strict_keys


def _string_array(value: object, *, location: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A string array with the required cardinality is required.",
            {"location": location},
        )
    result = [nonempty_string(item, location=f"{location}[]") for item in value]
    if len(result) != len(set(result)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_array_value",
            "Array values must be unique.",
            {"location": location},
        )
    return result


def validate_task_changes(
    value: object,
    spec_id: str,
    known_ids: set[str],
) -> None:
    if not isinstance(value, list) or not value:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_item_array",
            "changes must be non-empty.",
        )
    previous = 0
    for index, raw_change in enumerate(value):
        change = strict_keys(
            raw_change,
            location=f"changes[{index}]",
            required={"id", "spec_id", "date", "reason", "affected_ids", "edits"},
            optional={"plan_change_ids"},
        )
        match = re.fullmatch(r"TASK-CHANGE-(\d{3})", str(change["id"]))
        if not match or int(match.group(1)) <= previous:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_or_unsorted_id",
                "Invalid TASK change ID.",
            )
        previous = int(match.group(1))
        if change["spec_id"] != spec_id:
            raise WorkError(
                ExitCode.CONTRACT,
                "change_spec_mismatch",
                "Change spec_id mismatch.",
            )
        try:
            date.fromisoformat(nonempty_string(change["date"], location="change.date"))
        except ValueError as error:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_change_date",
                "Use YYYY-MM-DD.",
            ) from error
        nonempty_string(change["reason"], location="change.reason")
        affected = _string_array(change["affected_ids"], location="change.affected_ids")
        if any(item_id.split("/", 1)[0] not in known_ids for item_id in affected):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_reference",
                "Unknown affected ID.",
            )
        if "plan_change_ids" in change:
            for item_id in _string_array(
                change["plan_change_ids"], location="change.plan_change_ids"
            ):
                if not re.fullmatch(r"PLAN-CHANGE-\d{3}", item_id):
                    raise WorkError(
                        ExitCode.CONTRACT,
                        "invalid_reference",
                        "Invalid Plan change ID.",
                    )
        edits = change["edits"]
        if not isinstance(edits, list) or not edits:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_item_array",
                "Change edits must be non-empty.",
            )
        for edit_index, raw_edit in enumerate(edits):
            edit = strict_keys(
                raw_edit,
                location=f"changes[{index}].edits[{edit_index}]",
                required={"operation", "path"},
                optional={"before", "after"},
            )
            operation = edit["operation"]
            expected = {
                "add": {"operation", "path", "after"},
                "replace": {"operation", "path", "before", "after"},
                "remove": {"operation", "path", "before"},
            }
            if operation not in expected or set(edit) != expected[operation]:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_change_edit",
                    "Invalid change edit fields.",
                )
            path = nonempty_string(edit["path"], location="change.edit.path")
            if not path.startswith("/"):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_json_pointer",
                    "Change path must be a JSON Pointer.",
                )
