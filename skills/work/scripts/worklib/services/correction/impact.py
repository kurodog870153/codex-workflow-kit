from typing import Any

from ...models.common.errors import ExitCode, WorkError


def affected_task_ids(index: dict[str, Any], task_contract: dict[str, Any], *, task_id: str, invalidates_completion: bool) -> list[str]:
    if not invalidates_completion:
        return []
    row = next(item for item in index["tasks"] if item["id"] == task_id)
    if row["status"] != "completed":
        raise WorkError(ExitCode.WORKFLOW_STATE, "correction_create_target_not_completed", "Completion invalidation requires a completed target TASK.", None)
    affected = {task_id}
    changed = True
    while changed:
        changed = False
        for task in task_contract["tasks"]:
            if task["id"] not in affected and any(dependency in affected for dependency in task.get("dependencies", [])):
                affected.add(task["id"])
                changed = True
    return [item["id"] for item in index["tasks"] if item["id"] in affected and item["status"] == "completed"]
