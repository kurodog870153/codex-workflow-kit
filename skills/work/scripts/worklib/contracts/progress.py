"""Discussion memory, independent of formal specification readiness."""

from __future__ import annotations

import re
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import validate_requirement_id
from .validation import nonempty_string, strict_keys


FIELDS = (
    "schema", "requirement_id", "mode", "revision", "status", "title", "request",
    "current_task_id", "context", "source_status", "notes", "confirmed_decisions",
    "tentative", "open_questions", "next_discussion_point",
)


def validate_progress_contract(value: object) -> dict[str, Any]:
    progress = strict_keys(value, location="progress", required=set(FIELDS))
    if progress["schema"] != "work-discussion-progress/v1" or progress["status"] != "discussion_only":
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_schema", "Progress must be discussion-only memory.")
    validate_requirement_id(nonempty_string(progress["requirement_id"], location="requirement_id"))
    if progress["mode"] not in ("plan", "task"):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_mode", "Only Plan and Task discussions can be saved.")
    if type(progress["revision"]) is not int or progress["revision"] < 1:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "A positive integer revision is required.")
    task_id = progress["current_task_id"]
    if task_id is not None and (
        progress["mode"] != "task" or not isinstance(task_id, str)
        or not re.fullmatch(r"TASK-[0-9]{3}", task_id)
    ):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_task", "Only Task progress may identify a current TASK-NNN.")
    for field in ("title", "request", "next_discussion_point"):
        nonempty_string(progress[field], location=field)
    if not isinstance(progress["context"], dict):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_context", "Supplied context must be a JSON object.")
    for field in ("source_status", "notes", "tentative", "open_questions", "confirmed_decisions"):
        if not isinstance(progress[field], list):
            raise WorkError(ExitCode.CONTRACT, "invalid_progress_list", "Discussion entries must be arrays.", {"field": field})
        for number, item in enumerate(progress[field]):
            location = f"{field}[{number}]"
            if field == "confirmed_decisions":
                decision = strict_keys(item, location=location, required={"statement"}, optional={"rationale"})
                nonempty_string(decision["statement"], location=location + ".statement")
                if "rationale" in decision:
                    nonempty_string(decision["rationale"], location=location + ".rationale")
            else:
                nonempty_string(item, location=location)
    # Context is inert historical data. Do not validate live Plan, TASK, skills or
    # instruction snapshots here: unavailable or stale sources are why saving is needed.
    ordered = {field: progress[field] for field in FIELDS}
    return parse_json_contract(render_json_contract(ordered), source="progress candidate")
