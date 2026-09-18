"""Task collection diagnostic orchestration."""

from __future__ import annotations

from pathlib import Path

from ...services.attempt.validation import (
    build_initial_execution_index,
    validate_execution_index,
)
from ...services.task.structure import inspect_task_structure
from ...services.task.task_diagnostics import diagnose_task_collection as inspect_task_collection
from .collection import validate_task_collection_contract
from .index import render_task_index_contract, validate_task_index_contract
from .item import render_task_item_contract, validate_task_item_contract


def diagnose_task_collection(
    project_root: Path,
    user_config_root: str,
    raw_index_path: str,
    *,
    skill_roots=None,
    validate_plan_contract,
):
    return inspect_task_collection(
        project_root,
        user_config_root,
        raw_index_path,
        skill_roots=skill_roots,
        validate_plan_contract=validate_plan_contract,
        build_initial_execution_index=build_initial_execution_index,
        validate_execution_index=validate_execution_index,
        render_task_index_contract=render_task_index_contract,
        render_task_item_contract=render_task_item_contract,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_task_index_contract=validate_task_index_contract,
        validate_task_item_contract=validate_task_item_contract,
        inspect_task_structure=inspect_task_structure,
    )


__all__ = ["diagnose_task_collection"]
