from __future__ import annotations

import re
from typing import Any, ClassVar, Literal

from pydantic import Field

from ...protocol import (
    ATTEMPT_ID_PATTERN as ATTEMPT_ID_PATTERN_TEXT,
    TASK_ID_PATTERN as TASK_ID_PATTERN_TEXT,
)
from ..common.base import WorkContract


TASK_ID_PATTERN = re.compile(TASK_ID_PATTERN_TEXT)
ATTEMPT_ID_PATTERN = re.compile(ATTEMPT_ID_PATTERN_TEXT)
CORRECTION_ID_PATTERN = re.compile(r"^ATTEMPT-\d{3}-CORRECTION-\d{3}$")
TASK_INSTRUCTION_AUDIT_ID_PATTERN = re.compile(r"^TASK-INSTRUCTION-AUDIT-\d{3}$")
RECORD_ID_PATTERN = re.compile(r"^(?:CMD|OP|VAL)-\d{3}(?:#[1-9]\d*)?$")
TASK_STATUSES = {"pending", "in_progress", "pending_retry", "blocked", "completed", "cancelled"}


class ExecutionIndexContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-index/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "title", "task_spec_id",
        "task_collection_sha256", "task_index_sha256",
        "task_instructions_sha256", "hierarchy_selection_sha256",
        "skill_selection_sha256", "instruction_selection_manifest",
        "latest_task_instruction_audit", "lock",
        "overall_status", "tasks",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-execution-index/v1", "requirement_id": "example",
        "title": "Execution", "task_spec_id": "TASK-SPEC-001",
        "task_collection_sha256": "a" * 64, "task_index_sha256": "b" * 64,
        "task_instructions_sha256": "c" * 64,
        "hierarchy_selection_sha256": "d" * 64,
        "skill_selection_sha256": "e" * 64, "overall_status": "pending",
        "tasks": [{"id": "TASK-001", "status": "pending", "skill_id": None,
                   "task_item_sha256": "f" * 64, "instructions_sha256": "0" * 64}],
    }

    schema_: Literal["work-execution-index/v1"] = Field(alias="schema")
    requirement_id: Any
    title: Any
    task_spec_id: Any
    task_collection_sha256: Any
    task_index_sha256: Any
    task_instructions_sha256: Any
    hierarchy_selection_sha256: Any
    skill_selection_sha256: Any
    instruction_selection_manifest: Any | None = None
    latest_task_instruction_audit: Any | None = None
    lock: Any | None = None
    overall_status: Any
    tasks: Any
