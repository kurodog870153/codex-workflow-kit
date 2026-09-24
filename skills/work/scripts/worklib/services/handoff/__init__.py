"""Independent Handoff capabilities."""

from .build import build_discussion_handoff, build_handoff
from .execution import (
    BASE_EXECUTE_REFERENCES, BLOCKING_STOPPED_TYPES, RECOVERY_REFERENCE,
    find_task_row, require_index_identity, validate_execute_instruction_selection,
    validate_execution_identity,
)
from .fingerprint import plan_item_ids, task_affected_ids, task_fingerprints
from .validation import render_handoff_json_contract, validate_handoff_contract, validate_handoff_json_contract
from .verification import require_known_affected_ids, require_matching_handoff_source

__all__ = [
    "BASE_EXECUTE_REFERENCES", "BLOCKING_STOPPED_TYPES", "RECOVERY_REFERENCE",
    "build_discussion_handoff", "build_handoff", "find_task_row", "plan_item_ids", "render_handoff_json_contract",
    "require_index_identity", "require_known_affected_ids", "require_matching_handoff_source",
    "task_affected_ids", "task_fingerprints", "validate_execute_instruction_selection",
    "validate_execution_identity", "validate_handoff_contract", "validate_handoff_json_contract",
]
