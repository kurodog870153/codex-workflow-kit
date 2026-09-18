from __future__ import annotations

import re
from typing import Any

from ...models.common.validation import ContractValuePolicy
from ...technical.infrastructure.json_contract import parse_json_contract, render_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.common.identifiers import IdentifierPolicy
from ...models.progress import FIELDS

nonempty_string = ContractValuePolicy.nonempty_string
sha256 = ContractValuePolicy.sha256
strict_keys = ContractValuePolicy.strict_keys
validate_sha256 = sha256
validate_strict_keys = strict_keys


def parse_progress_request(raw: bytes, *, source: str) -> dict[str, Any]:
    return parse_json_contract(raw, source=source)


def validate_progress_contract(value: object) -> dict[str, Any]:
    progress = strict_keys(value, location="progress", required=set(FIELDS))
    if progress["schema"] != "work-discussion-progress/v1" or progress["status"] != "discussion_only":
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_schema", "Progress must be discussion-only memory.")
    IdentifierPolicy.requirement_id(nonempty_string(progress["requirement_id"], location="requirement_id"))
    if progress["mode"] not in ("plan", "task"):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_mode", "Only Plan and Task discussions can be saved.")
    if type(progress["revision"]) is not int or progress["revision"] < 1:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "A positive integer revision is required.")
    task_id = progress["current_task_id"]
    if task_id is not None and (progress["mode"] != "task" or not isinstance(task_id, str) or not re.fullmatch(r"TASK-[0-9]{3}", task_id)):
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
    ordered = {field: progress[field] for field in FIELDS}
    return parse_json_contract(render_json_contract(ordered), source="progress candidate")
