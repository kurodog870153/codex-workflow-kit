from __future__ import annotations

from typing import Any

from ...protocol import BLOCKING_STOPPED_TYPES
from ...models.common.errors import ExitCode, WorkError


BASE_EXECUTE_REFERENCES = ["execute.general.execution-records"]
RECOVERY_REFERENCE = "execute.general.execution-recovery"
def find_task_row(index: dict[str, Any], task_id: str) -> dict[str, Any]:
    try:
        return next(item for item in index["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "record_begin_task_not_found",
            "The requested TASK is not present in the execution index.",
            {"task_id": task_id},
        ) from error


def validate_execution_identity(*, task_contract, task_validation, index, attempt, task_id):
    source_fingerprints = {
        "task_collection_sha256": task_validation["task_collection_sha256"],
        "task_index_sha256": task_validation["task_index_sha256"],
    }
    expected_index = {
        "requirement_id": task_contract["requirement_id"],
        "task_spec_id": task_contract["spec_id"],
        **source_fingerprints,
        "task_instructions_sha256": task_validation["instructions_sha256"],
        "hierarchy_selection_sha256": task_validation["hierarchy_selection_sha256"],
    }
    observed_index = {field: index[field] for field in expected_index}
    if observed_index != expected_index:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "record_begin_index_identity_mismatch", "The execution index does not match the formal TASK identity.", {"expected": expected_index, "actual": observed_index})
    expected_ids = task_validation["task_ids"]
    if [item["id"] for item in index["tasks"]] != expected_ids:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "record_begin_index_task_set_mismatch", "The execution index TASK set does not match the formal TASK.")
    row = find_task_row(index, task_id)
    task_instructions = task_validation["task_instructions_sha256"]
    if row["instructions_sha256"] != task_instructions[task_id]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "record_begin_task_instructions_mismatch", "The target TASK instruction fingerprint is stale.")
    expected_attempt = {
        "task_spec_id": task_contract["spec_id"], "task_id": task_id,
        **source_fingerprints,
        "task_item_sha256": task_validation["task_item_sha256"][task_id],
        "task_instructions_sha256": task_instructions[task_id],
        "hierarchy_selection_sha256": task_validation["hierarchy_selection_sha256"],
    }
    observed_attempt = {field: attempt[field] for field in expected_attempt}
    if observed_attempt != expected_attempt:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "record_begin_attempt_identity_mismatch", "The active Attempt does not match the formal TASK identity.", {"expected": expected_attempt, "actual": observed_attempt})
    return row


def require_index_identity(index, task, task_validation):
    expected_identity = {
        "requirement_id": task["requirement_id"], "task_spec_id": task["spec_id"],
        "task_collection_sha256": task_validation["task_collection_sha256"],
        "task_index_sha256": task_validation["task_index_sha256"],
        "task_instructions_sha256": task_validation["instructions_sha256"],
        "hierarchy_selection_sha256": task_validation["hierarchy_selection_sha256"],
    }
    observed_identity = {key: index[key] for key in expected_identity}
    if observed_identity != expected_identity:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "execute_preflight_index_identity_mismatch", "The execution index does not match the formal TASK identity.", {"expected": expected_identity, "observed": observed_identity})
    task_instructions = task_validation["task_instructions_sha256"]
    expected_ids = task_validation["task_ids"]
    observed_ids = [item["id"] for item in index["tasks"]]
    if observed_ids != expected_ids:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "execute_preflight_index_task_set_mismatch", "The execution index TASK set does not match the formal TASK document.", {"expected": expected_ids, "observed": observed_ids})
    rows = {item["id"]: item for item in index["tasks"]}
    mismatches = {
        task_id: {"expected": task_instructions[task_id], "observed": rows[task_id]["instructions_sha256"]}
        for task_id in expected_ids
        if rows[task_id]["instructions_sha256"] != task_instructions[task_id]
    }
    if mismatches:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "execute_preflight_task_instructions_mismatch", "The execution index per-TASK instruction fingerprints are stale.", {"tasks": mismatches})
    return rows


def validate_execute_instruction_selection(task, attempt, current, *, operation):
    selection = task["instruction_selection"]
    if current["selected_paths"] != selection["selected_paths"] or current["resolved_paths"] != selection["resolved_paths"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, f"{operation}_execute_instruction_hierarchy_mismatch", "The current Execute hierarchy does not match the target TASK.")
    if current["instructions_sha256"] != attempt["execute_instructions_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, f"{operation}_execute_instructions_changed", "The Execute instruction fingerprint changed after Attempt start.", {"expected": attempt["execute_instructions_sha256"], "actual": current["instructions_sha256"]})
    return current
