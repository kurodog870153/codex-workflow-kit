"""Independent Task capabilities."""

from .changes_validation import validate_task_changes
from .draft.validation import resolve_task_dependencies
from .document import parse_task_contract, render_ordered_task_contract
from .ordering import (
    order_task_contract,
    order_task_index_contract,
    order_task_item_contract,
)
from .path import task_path_candidates
from .structure import (
    TASK_OPTIONAL,
    TASK_REQUIRED,
    TOP_OPTIONAL,
    TOP_REQUIRED,
    inspect_task_structure,
    pointer,
)
from .storage import read_project_task_source, read_task_index, read_task_item, task_collection_item_names, task_path_existence
from .semantic_validation import validate_task_contract as validate_task_semantics

__all__ = [
    "validate_task_changes",
    "resolve_task_dependencies",
    "render_ordered_task_contract",
    "parse_task_contract",
    "order_task_contract",
    "order_task_index_contract",
    "order_task_item_contract",
    "task_path_candidates",
    "TASK_OPTIONAL",
    "TASK_REQUIRED",
    "TOP_OPTIONAL",
    "TOP_REQUIRED",
    "inspect_task_structure",
    "pointer",
    "read_task_index",
    "read_project_task_source",
    "read_task_item",
    "task_path_existence",
    "task_collection_item_names",
    "validate_task_semantics",
]
