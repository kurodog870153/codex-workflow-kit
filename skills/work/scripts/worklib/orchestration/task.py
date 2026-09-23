"""Task CLI workflow spanning Task and Specification business services."""

from __future__ import annotations

import argparse
from pathlib import Path

from ..business_services.specification import (
    execution_history_fingerprints,
    prepare_specification as prepare_specification_business,
    preview_specification_migration,
    preview_specification_reconciliation,
    publish_specification_migration,
    publish_specification_reconciliation,
    rebuild_execution_index,
    update_specification as update_specification_business,
    verify_specification as verify_specification_business,
)
from ..business_services.specification.migration import (
    preview_specification_migration as preview_specification_migration_business,
    publish_specification_migration as publish_specification_migration_business,
)
from ..business_services.specification.reconciliation import (
    preview_specification_reconciliation as preview_specification_reconciliation_business,
    publish_specification_reconciliation as publish_specification_reconciliation_business,
)
from ..business_services.plan import validate_plan_contract
from ..business_services.instruction import (
    build_instruction_selection,
    build_task_document_instruction_selection,
)
from ..business_services.task.collection import validate_task_collection_contract
from ..business_services.task.diagnostics import (
    diagnose_task_collection as diagnose_task_collection_business,
)
from ..business_services.task.index import render_task_index_contract
from ..business_services.task.item import (
    render_task_item_contract,
    validate_task_item_contract,
)
from ..business_services.task.repair import (
    prepare_task_repair as prepare_task_repair_business,
    repair_task as repair_task_business,
)
from ..business_services.task.draft_assembly import (
    assemble_task_drafts as assemble_task_drafts_business,
    create_task_from_drafts as create_task_from_drafts_business,
)
from ..business_services.task.draft_prepare import (
    initialize_task_planning_request as initialize_task_planning_request_business,
    prepare_task_planning_request as prepare_task_planning_request_business,
)
from ..business_services.task.draft_request import save_task_draft_request as save_task_draft_request_business
from ..business_services.task.draft_source import check_task_draft_sources as check_task_draft_sources_business
from ..business_services.task.draft_source_update import update_task_draft_sources as update_task_draft_sources_business


def preview_specification_migration(*args, **kwargs):
    return preview_specification_migration_business(
        *args,
        **kwargs,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_plan_contract=validate_plan_contract,
    )


def publish_specification_migration(*args, **kwargs):
    return publish_specification_migration_business(
        *args,
        **kwargs,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_plan_contract=validate_plan_contract,
    )


def preview_specification_reconciliation(*args, **kwargs):
    return preview_specification_reconciliation_business(
        *args,
        **kwargs,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_plan_contract=validate_plan_contract,
    )


def publish_specification_reconciliation(*args, **kwargs):
    return publish_specification_reconciliation_business(
        *args,
        **kwargs,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_plan_contract=validate_plan_contract,
    )
from ..business_services.task.workflow import (
    TaskRequestInput,
    execute_task_command as execute_task_business_command,
)


class TaskOperations:
    render_task_index_contract = staticmethod(render_task_index_contract)
    render_task_item_contract = staticmethod(render_task_item_contract)
    validate_task_collection_contract = staticmethod(validate_task_collection_contract)
    validate_task_item_contract = staticmethod(validate_task_item_contract)


class TaskDraftOperations:
    build_instruction_selection = staticmethod(build_instruction_selection)
    build_task_document_instruction_selection = staticmethod(build_task_document_instruction_selection)
    validate_plan_contract = staticmethod(validate_plan_contract)


def _draft(operation):
    def bound(*args, **kwargs):
        return operation(*args, **kwargs, operations=TaskDraftOperations)
    return bound


assemble_task_drafts = _draft(assemble_task_drafts_business)
check_task_draft_sources = _draft(check_task_draft_sources_business)
create_task_from_drafts = _draft(create_task_from_drafts_business)
initialize_task_planning_request = _draft(initialize_task_planning_request_business)
prepare_task_planning_request = _draft(prepare_task_planning_request_business)
save_task_draft_request = _draft(save_task_draft_request_business)
update_task_draft_sources = _draft(update_task_draft_sources_business)


def prepare_specification(*args, **kwargs):
    return prepare_specification_business(*args, **kwargs, task_operations=TaskOperations)


def update_specification(*args, **kwargs):
    return update_specification_business(*args, **kwargs, task_operations=TaskOperations)


def verify_specification(*args, **kwargs):
    return verify_specification_business(*args, **kwargs, task_operations=TaskOperations)


def diagnose_task_collection(*args, **kwargs):
    return diagnose_task_collection_business(
        *args,
        **kwargs,
        validate_plan_contract=validate_plan_contract,
    )


def repair_task(*args, **kwargs):
    return repair_task_business(
        *args,
        **kwargs,
        execution_history_fingerprints=execution_history_fingerprints,
        rebuild_execution_index=rebuild_execution_index,
        diagnose_task_collection=diagnose_task_collection,
    )


def prepare_task_repair(*args, **kwargs):
    return prepare_task_repair_business(
        *args,
        **kwargs,
        execution_history_fingerprints=execution_history_fingerprints,
        rebuild_execution_index=rebuild_execution_index,
        diagnose_task_collection=diagnose_task_collection,
    )


def execute_task_command(
    arguments: argparse.Namespace,
    project_root: Path,
    request: TaskRequestInput | None,
) -> dict[str, object]:
    return execute_task_business_command(
        arguments,
        project_root,
        request,
        prepare_specification=prepare_specification,
        preview_specification_migration=preview_specification_migration,
        preview_specification_reconciliation=preview_specification_reconciliation,
        publish_specification_migration=publish_specification_migration,
        publish_specification_reconciliation=publish_specification_reconciliation,
        update_specification=update_specification,
        verify_specification=verify_specification,
        prepare_task_repair=prepare_task_repair,
        repair_task=repair_task,
        diagnose_task_collection=diagnose_task_collection,
        draft_operations=TaskDraftOperations,
    )


__all__ = [
    "TaskRequestInput",
    "execute_task_command",
    "diagnose_task_collection",
    "prepare_task_repair",
    "repair_task",
    "assemble_task_drafts",
    "check_task_draft_sources",
    "create_task_from_drafts",
    "initialize_task_planning_request",
    "prepare_task_planning_request",
    "save_task_draft_request",
    "update_task_draft_sources",
]
