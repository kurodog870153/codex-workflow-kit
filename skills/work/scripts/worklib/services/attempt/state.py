from __future__ import annotations

import copy
from typing import Any

from ...protocol import BLOCKING_STOPPED_TYPES


def lock_index(index: dict[str, Any], *, lock: dict[str, str]) -> dict[str, Any]:
    result = copy.deepcopy(index)
    result["lock"] = lock
    return result


def start_index(
    index: dict[str, Any], *, task_id: str, attempt_id: str, overall_status: str
) -> dict[str, Any]:
    result = copy.deepcopy(index)
    row = next(item for item in result["tasks"] if item["id"] == task_id)
    row["status"] = "in_progress"
    row["latest_attempt"] = attempt_id
    row.pop("status_reason", None)
    result["overall_status"] = overall_status
    return result


def closed_task_status(request: dict[str, Any]) -> str:
    if request["status"] == "completed":
        return "completed"
    if request["status"] == "blocked":
        return "blocked"
    if request["final_type"] in BLOCKING_STOPPED_TYPES:
        return "blocked"
    return "pending_retry"


def close_index(
    index: dict[str, Any], *, task_id: str, attempt_id: str,
    task_status: str, overall_status: str,
) -> dict[str, Any]:
    result = copy.deepcopy(index)
    row = next(item for item in result["tasks"] if item["id"] == task_id)
    row["status"] = task_status
    if task_status == "completed":
        row.pop("status_reason", None)
    else:
        row["status_reason"] = {"kind": "attempt", "ref": attempt_id}
    result.pop("lock")
    result["overall_status"] = overall_status
    return result
