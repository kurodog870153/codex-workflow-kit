"""Task business services."""

from importlib import import_module
from typing import Any


_EXPORTS = {
    "assemble_task_drafts": ".draft_assembly",
    "check_task_draft_sources": ".draft_source",
    "create_task_artifacts": ".creation",
    "create_task_from_drafts": ".draft_assembly",
    "initialize_task_planning_request": ".draft_prepare",
    "load_task_closure": ".io",
    "load_task_collection": ".io",
    "load_task_execution_context": ".io",
    "prepare_task_collection_create": ".creation",
    "prepare_task_planning_request": ".draft_prepare",
    "read_task_draft": "worklib.services.task.draft.storage",
    "read_task_planning_index": "worklib.services.task.draft.storage",
    "recover_task_create": ".creation",
    "recover_task_planning": "worklib.services.task.draft.storage",
    "render_task_contract": ".document",
    "render_task_index_contract": ".index",
    "render_task_item_contract": ".item",
    "save_task_draft_request": ".draft_request",
    "save_task_planning": "worklib.services.task.draft.storage",
    "task_draft_status": ".draft_status",
    "update_task_draft_sources": ".draft_source_update",
    "update_task_planning_list": ".draft_list",
    "validate_task_collection_contract": ".collection",
    "validate_task_contract": ".semantic",
    "validate_task_index_contract": ".index",
    "validate_task_item_contract": ".item",
}

__all__ = sorted(_EXPORTS)


def __getattr__(name: str) -> Any:
    module_name = _EXPORTS.get(name)
    if module_name is None:
        raise AttributeError(name)
    value = getattr(import_module(module_name, __name__), name)
    globals()[name] = value
    return value
