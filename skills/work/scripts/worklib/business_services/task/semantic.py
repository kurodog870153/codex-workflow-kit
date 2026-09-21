from __future__ import annotations

from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.skill import SkillRoot
from ...services.instruction.history import stored_document_selection
from ...services.instruction.root import instruction_root
from ...services.task.changes_validation import validate_task_changes
from ...services.task.draft.validation import resolve_task_dependencies
from ...services.task.document import parse_task_contract
from ...services.task.ordering import order_task_contract
from ...services.task.path import task_path_candidates
from ...services.task.semantic_validation import (
    validate_task_context_plan,
    validate_task_context_shape,
    validate_task_contract as validate_semantics,
)
from ...services.task.storage import read_project_task_source, task_path_existence
from ...services.task.structure import TASK_OPTIONAL, TASK_REQUIRED, TOP_OPTIONAL, TOP_REQUIRED
from .hierarchy import validate_task_hierarchy_paths
from .instruction import validate_task_document_selection
from .plan import validate_task_source_plan


def _context(
    contract: dict[str, Any],
    *,
    project_root: Path,
    user_config_root: str,
    validate_file_state: bool,
    skill_roots: list[SkillRoot] | None,
    source_plan_raw: bytes | None,
    historical_work_sources: bool,
    reviewed_source_plan_binding: tuple[str, str] | None,
) -> dict[str, Any]:
    tasks = validate_task_context_shape(
        contract,
        top_required=TOP_REQUIRED,
        top_optional=TOP_OPTIONAL,
        task_required=TASK_REQUIRED,
        task_optional=TASK_OPTIONAL,
        historical_work_sources=historical_work_sources,
        source_plan_raw=source_plan_raw,
        reviewed_source_plan_binding=reviewed_source_plan_binding,
    )
    artifacts = contract["artifacts"]
    if source_plan_raw is None:
        _, _, plan_raw = read_project_task_source(
            project_root, artifacts["plan"], field="plan_path"
        )
    else:
        plan_raw = source_plan_raw
    plan_validation = validate_task_source_plan(
        plan_raw,
        source="TASK source Plan",
        actual_plan_path=artifacts["plan"],
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
        _historical_work_sources=historical_work_sources,
        _allow_task_index=True,
    )
    plan_contract = parse_task_contract(plan_raw, source="TASK source Plan")
    task_ids, dependencies = validate_task_context_plan(
        contract,
        tasks,
        plan_contract=plan_contract,
        plan_validation=plan_validation,
        reviewed_source_plan_binding=reviewed_source_plan_binding,
    )
    selections = [
        task.get("instruction_selection")
        for task in tasks
        if isinstance(task, dict)
    ]
    work_root = instruction_root()
    document_selection = (
        stored_document_selection(contract.get("instruction_selection"), selections)
        if historical_work_sources
        else validate_task_document_selection(
            contract.get("instruction_selection"), selections, skill_root=work_root
        )
    )
    confirmed_selection = plan_contract.get("hierarchy_selection")
    for task in tasks:
        if not isinstance(task, dict):
            continue
        selection = task.get("instruction_selection")
        if not isinstance(selection, dict):
            continue
        validate_task_hierarchy_paths(
            selection.get("selected_paths"),
            confirmed_selection=confirmed_selection,
            skill_root=work_root,
            location=f"{task.get('id')}.instruction_selection.selected_paths",
        )
    topo_order, ancestors = resolve_task_dependencies(task_ids, dependencies)
    decisions = contract.get("decisions")
    known_ids = set(task_ids) | {
        decision["id"]
        for decision in (decisions if isinstance(decisions, list) else [])
        if isinstance(decision, dict) and isinstance(decision.get("id"), str)
    }
    changes_validated = "changes" not in contract
    if "changes" in contract:
        validate_task_changes(contract["changes"], str(contract.get("spec_id")), known_ids)
        changes_validated = True
    candidates = task_path_candidates(contract, project_root)
    return {
        "plan_raw": plan_raw,
        "plan_validation": plan_validation,
        "document_selection": document_selection,
        "topo_order": topo_order,
        "ancestors": ancestors,
        "changes_validated": changes_validated,
        "path_existence": (
            task_path_existence(candidates) if validate_file_state else {}
        ),
        "ordered_contract": order_task_contract(contract),
        "top_required": TOP_REQUIRED,
        "top_optional": TOP_OPTIONAL,
        "task_required": TASK_REQUIRED,
        "task_optional": TASK_OPTIONAL,
        "historical_work_sources": historical_work_sources,
    }


def validate_task_contract(
    raw: bytes,
    *,
    source: str,
    actual_task_path: str,
    project_root: Path,
    user_config_root: str,
    validate_file_state: bool = True,
    skill_roots: list[SkillRoot] | None = None,
    _source_plan_raw: bytes | None = None,
    _historical_work_sources: bool = False,
    _reviewed_source_plan_binding: tuple[str, str] | None = None,
) -> dict[str, object]:
    try:
        contract = parse_task_contract(raw, source=source)
        context = _context(
            contract,
            project_root=project_root,
            user_config_root=user_config_root,
            validate_file_state=validate_file_state,
            skill_roots=skill_roots,
            source_plan_raw=_source_plan_raw,
            historical_work_sources=_historical_work_sources,
            reviewed_source_plan_binding=_reviewed_source_plan_binding,
        )
        return validate_semantics(
            raw,
            source=source,
            actual_task_path=actual_task_path,
            project_root=project_root,
            context=context,
            validate_file_state=validate_file_state,
            _reviewed_source_plan_binding=_reviewed_source_plan_binding,
        )
    except WorkError:
        raise
    except (TypeError, KeyError, ValueError, RecursionError) as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_task_value",
            "The TASK contains a value the contract cannot validate.",
            {"exception_type": type(error).__name__},
        ) from error


__all__ = ["validate_task_contract"]
