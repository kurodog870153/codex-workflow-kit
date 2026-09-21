from __future__ import annotations

import re
from typing import Any, ClassVar, Literal

from pydantic import Field

from ...protocol import (
    SHA256_PATTERN as SHA256_PATTERN_TEXT,
    TASK_ID_PATTERN as TASK_ID_PATTERN_TEXT,
)
from ..common.base import WorkContract
from .authorization import AttemptAuthorizationContract


ATTEMPT_PATTERN = re.compile(r"^ATTEMPT-(\d{3})$")
TASK_PATTERN = re.compile(TASK_ID_PATTERN_TEXT)
TASK_SPEC_PATTERN = re.compile(r"^TASK-SPEC-\d{3}$")
SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)
TIMESTAMP_PATTERN = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}[+-]\d{2}:\d{2}$"
)
RECORD_PATTERN = re.compile(r"^(CMD|OP|VAL)-\d{3}(?:#([1-9]\d*))?$")
ATTEMPT_STATUSES = {"in_progress", "completed", "stopped", "blocked"}
STOPPED_TYPES = {
    "specification_defect", "instructions_changed", "validation_failed",
    "unexpected_change", "external_operation_failed", "user_stopped", "other",
}
BLOCKED_TYPES = {"environment", "external_service", "permission", "required_input", "other"}
ROOT_REQUIRED = {
    "schema", "attempt_id", "task_spec_id", "task_id", "skill_id", "status",
    "task_collection_sha256", "task_index_sha256", "task_item_sha256",
    "task_instructions_sha256", "execute_instructions_sha256",
    "hierarchy_selection_sha256", "execute_skill_selection_sha256",
    "authorization", "authorization_sha256", "started_at", "records",
}
ROOT_OPTIONAL = {
    "continued_from", "carried_records", "modified_files", "execution_deviations",
    "overall_result", "final_type", "reason", "closing_authorization_evidence",
    "ended_at",
}
ROOT_ORDER = (
    "schema", "attempt_id", "task_spec_id", "task_id", "skill_id", "status",
    "task_collection_sha256", "task_index_sha256", "task_item_sha256",
    "task_instructions_sha256", "execute_instructions_sha256",
    "hierarchy_selection_sha256", "execute_skill_selection_sha256",
    "authorization", "authorization_sha256", "started_at", "continued_from",
    "carried_records", "modified_files", "execution_deviations", "records",
    "overall_result", "final_type", "reason", "closing_authorization_evidence",
    "ended_at",
)


class AttemptContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = ROOT_ORDER
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-attempt/v1", "attempt_id": "ATTEMPT-001",
        "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001",
        "skill_id": None, "status": "in_progress",
        "task_collection_sha256": "a" * 64, "task_index_sha256": "b" * 64,
        "task_item_sha256": "c" * 64, "task_instructions_sha256": "d" * 64,
        "execute_instructions_sha256": "e" * 64,
        "hierarchy_selection_sha256": "f" * 64,
        "execute_skill_selection_sha256": "0" * 64,
        "authorization": AttemptAuthorizationContract.contract_example,
        "authorization_sha256": "30a8dd5343d43861e6a2f0846add3362ed674ca0be96dcdb4020d9004e6ff7aa",
        "started_at": "2026-09-01T10:00+08:00", "records": [],
    }

    schema_: Literal["work-attempt/v1"] = Field(alias="schema")
    attempt_id: Any
    task_spec_id: Any
    task_id: Any
    skill_id: Any
    status: Any
    task_collection_sha256: Any
    task_index_sha256: Any
    task_item_sha256: Any
    task_instructions_sha256: Any
    execute_instructions_sha256: Any
    hierarchy_selection_sha256: Any
    execute_skill_selection_sha256: Any
    authorization: AttemptAuthorizationContract
    authorization_sha256: Any
    started_at: Any
    continued_from: Any | None = None
    carried_records: Any | None = None
    modified_files: Any | None = None
    execution_deviations: Any | None = None
    records: Any
    overall_result: Any | None = None
    final_type: Any | None = None
    reason: Any | None = None
    closing_authorization_evidence: Any | None = None
    ended_at: Any | None = None


class AttemptValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "attempt_id", "task_spec_id", "task_id", "status",
        "record_count", "result",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-attempt-validation/v1", "attempt_id": "ATTEMPT-001",
        "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001",
        "status": "in_progress", "record_count": 0, "result": "valid",
    }

    schema_: Literal["work-attempt-validation/v1"] = Field(alias="schema")
    attempt_id: str
    task_spec_id: str
    task_id: str
    status: Literal["in_progress", "completed", "stopped", "blocked"]
    record_count: int
    result: Literal["valid"]
