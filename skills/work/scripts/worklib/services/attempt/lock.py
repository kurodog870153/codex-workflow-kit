def build_execution_lock(
    *, task_id: str, attempt_id: str, execute_instructions_sha256: str
) -> dict[str, str]:
    return {
        "kind": "execution",
        "task_id": task_id,
        "attempt_id": attempt_id,
        "execute_instructions_sha256": execute_instructions_sha256,
    }
