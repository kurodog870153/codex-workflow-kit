from __future__ import annotations

from typing import Any

from ...models.common.errors import ExitCode, WorkError


def require_index_identity(
    index: dict[str, Any],
    task: dict[str, Any],
    task_validation: dict[str, object],
) -> dict[str, dict[str, Any]]:
    expected_identity = {
        "requirement_id": task["requirement_id"],
        "task_spec_id": task["spec_id"],
        "task_collection_sha256": task_validation["task_collection_sha256"],
        "task_index_sha256": task_validation["task_index_sha256"],
        "task_instructions_sha256": task_validation["instructions_sha256"],
        "hierarchy_selection_sha256": task_validation["hierarchy_selection_sha256"],
    }
    observed_identity = {key: index[key] for key in expected_identity}
    if observed_identity != expected_identity:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            {"expected": expected_identity, "observed": observed_identity},
        )
    task_instructions = task_validation["task_instructions_sha256"]
    assert isinstance(task_instructions, dict)
    expected_ids = task_validation["task_ids"]
    observed_ids = [item["id"] for item in index["tasks"]]
    if observed_ids != expected_ids:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK document.",
            {"expected": expected_ids, "observed": observed_ids},
        )
    rows = {item["id"]: item for item in index["tasks"]}
    mismatches = {
        task_id: {
            "expected": task_instructions[task_id],
            "observed": rows[task_id]["instructions_sha256"],
        }
        for task_id in expected_ids
        if rows[task_id]["instructions_sha256"] != task_instructions[task_id]
    }
    if mismatches:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_task_instructions_mismatch",
            "The execution index per-TASK instruction fingerprints are stale.",
            {"tasks": mismatches},
        )
    return rows
