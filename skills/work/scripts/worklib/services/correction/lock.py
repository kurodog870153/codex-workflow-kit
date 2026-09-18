from typing import Any


def build_correction_lock(*, task_id: str, attempt_id: str, correction_id: str, execute_instructions_sha256: str, invalidates_completion: bool, affected_task_ids: list[str]) -> dict[str, Any]:
    return {"kind": "correction", "task_id": task_id, "attempt_id": attempt_id, "correction_id": correction_id, "execute_instructions_sha256": execute_instructions_sha256, "invalidates_completion": invalidates_completion, "affected_task_ids": affected_task_ids}
