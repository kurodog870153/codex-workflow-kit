from __future__ import annotations

from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def read_contract(path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = read_raw(path)
    contract = parse_json_contract(raw, source=str(path))
    return raw, contract


def find_task_row(index: dict[str, Any], task_id: str) -> dict[str, Any]:
    try:
        return next(item for item in index["tasks"] if item["id"] == task_id)
    except StopIteration as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "record_begin_task_not_found",
            "The requested TASK is not present in the execution index.",
            {"task_id": task_id},
        ) from error


def validate_execution_identity(
    *,
    task_contract: dict[str, Any],
    task_validation: dict[str, object],
    index: dict[str, Any],
    attempt: dict[str, Any],
    task_id: str,
) -> dict[str, Any]:
    expected_index = {
        "requirement_id": task_contract["requirement_id"],
        "task_spec_id": task_contract["spec_id"],
        "task_sha256": task_validation["task_sha256"],
        "task_instructions_sha256": task_validation["instructions_sha256"],
        "hierarchy_selection_sha256": task_validation[
            "hierarchy_selection_sha256"
        ],
    }
    observed_index = {field: index[field] for field in expected_index}
    if observed_index != expected_index:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            expected=expected_index,
            actual=observed_index,
        )
    expected_ids = [item["id"] for item in task_contract["tasks"]]
    if [item["id"] for item in index["tasks"]] != expected_ids:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK.",
        )
    row = find_task_row(index, task_id)
    task_instructions = task_validation["task_instructions_sha256"]
    assert isinstance(task_instructions, dict)
    if row["instructions_sha256"] != task_instructions[task_id]:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_task_instructions_mismatch",
            "The target TASK instruction fingerprint is stale.",
        )
    expected_attempt = {
        "task_spec_id": task_contract["spec_id"],
        "task_id": task_id,
        "task_sha256": task_validation["task_sha256"],
        "task_instructions_sha256": task_instructions[task_id],
        "hierarchy_selection_sha256": task_validation[
            "hierarchy_selection_sha256"
        ],
    }
    observed_attempt = {field: attempt[field] for field in expected_attempt}
    if observed_attempt != expected_attempt:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            "record_begin_attempt_identity_mismatch",
            "The active Attempt does not match the formal TASK identity.",
            expected=expected_attempt,
            actual=observed_attempt,
        )
    return row
