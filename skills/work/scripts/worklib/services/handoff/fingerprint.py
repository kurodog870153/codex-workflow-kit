from __future__ import annotations

from ...models.plan import ID_PREFIXES


def plan_item_ids(plan):
    return {item["id"] for group in ID_PREFIXES for item in plan.get(group, [])}


def task_fingerprints(validation, task_id=None):
    result = {
        "task_collection_sha256": validation["task_collection_sha256"],
        "task_index_sha256": validation["task_index_sha256"],
    }
    if task_id is not None:
        result["task_item_sha256"] = validation["task_item_sha256"][task_id]
    return result


def task_affected_ids(plan, task, task_id):
    known_ids = plan_item_ids(plan) | {entry["id"] for entry in task["tasks"]} | {
        entry["id"] for entry in task.get("decisions", [])
    }
    if task_id is not None:
        selected = next(entry for entry in task["tasks"] if entry["id"] == task_id)
        for group in ("steps", "validations", "commands", "operations", "files", "inputs", "decisions", "risks"):
            known_ids.update(item["id"] for item in selected.get(group, []))
    return known_ids

