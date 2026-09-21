import copy
from typing import Any


def corrected_index(index: dict[str, Any], *, task_id: str, correction_id: str, affected_task_ids: list[str], overall_status: str) -> dict[str, Any]:
    target = copy.deepcopy(index)
    target.pop("lock", None)
    target_row = next(item for item in target["tasks"] if item["id"] == task_id)
    target_row["latest_correction"] = correction_id
    affected = set(affected_task_ids)
    for row in target["tasks"]:
        if row["id"] in affected:
            row["status"] = "pending_retry"
            row["status_reason"] = {"kind": "correction", "ref": correction_id}
    target["overall_status"] = overall_status
    return target
