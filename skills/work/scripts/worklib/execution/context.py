from __future__ import annotations

from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..artifacts.task_collection import (
    load_task_execution_context,
    task_artifact_format,
)


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


def load_lifecycle_task_context(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    task_id: str,
    skill_roots: object,
    v1_validator: object,
) -> tuple[str, Path, dict[str, Any], dict[str, object]]:
    normalized, path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    if task_artifact_format(normalized) == "v2":
        context = load_task_execution_context(
            project_root,
            user_config_root,
            normalized,
            task_id,
            skill_roots=skill_roots,
        )
        contract = context["contract"]
        validation = context["validation"]
        assert isinstance(contract, dict) and isinstance(validation, dict)
        return normalized, path, contract, validation
    raw = read_raw(path)
    validation = v1_validator(
        raw,
        source=str(path),
        actual_task_path=normalized,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=False,
        skill_roots=skill_roots,
    )
    contract = parse_json_contract(raw, source=str(path))
    return normalized, path, contract, validation


def validate_execution_identity(
    *,
    task_contract: dict[str, Any],
    task_validation: dict[str, object],
    index: dict[str, Any],
    attempt: dict[str, Any],
    task_id: str,
) -> dict[str, Any]:
    is_collection = task_validation.get("schema") in {
        "work-task-collection-validation/v2",
        "work-task-execution-validation/v2",
    }
    source_fingerprints = (
        {
            "task_collection_sha256": task_validation["task_collection_sha256"],
            "task_index_sha256": task_validation["task_index_sha256"],
        }
        if is_collection
        else {"task_sha256": task_validation["task_sha256"]}
    )
    expected_index = {
        "requirement_id": task_contract["requirement_id"],
        "task_spec_id": task_contract["spec_id"],
        **source_fingerprints,
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
    expected_ids = (
        task_validation["task_ids"]
        if is_collection
        else [item["id"] for item in task_contract["tasks"]]
    )
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
        **source_fingerprints,
        **(
            {"task_item_sha256": task_validation["task_item_sha256"][task_id]}
            if is_collection
            else {}
        ),
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
