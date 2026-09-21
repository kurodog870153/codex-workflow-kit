"""Correction data models."""

from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from ..common.base import WorkContract


class CorrectionCreateRequestContract(WorkContract):
    contract_id: ClassVar[str] = 'work-correction-create-request/v1'
    contract_kind: ClassVar[Literal['request']] = 'request'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'target_attempt_id', 'field', 'correct_value', 'reason', 'invalidates_completion')
    schema_: Literal['work-correction-create-request/v1'] = Field(alias='schema')
    target_attempt_id: str
    field: str
    correct_value: str
    reason: str
    invalidates_completion: bool


class CorrectionCreateContract(WorkContract):
    contract_id: ClassVar[str] = 'work-correction-create/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'task_id', 'attempt_id', 'correction_id', 'correction_path', 'index_path', 'affected_task_ids', 'lock_status')
    schema_: Literal['work-correction-create/v1'] = Field(alias='schema')
    task_id: str
    attempt_id: str
    correction_id: str
    correction_path: str
    index_path: str
    affected_task_ids: list[str]
    lock_status: Literal['released']


CorrectionCreateRequestContract.contract_example = {
    "schema": "work-correction-create-request/v1",
    "target_attempt_id": "ATTEMPT-001",
    "field": "records[0].outcome",
    "correct_value": "passed",
    "reason": "Correct the recorded outcome.",
    "invalidates_completion": True,
}


CorrectionCreateContract.contract_example = {
    "schema": "work-correction-create/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "correction_id": "ATTEMPT-001-CORRECTION-001",
    "correction_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/corrections/ATTEMPT-001-CORRECTION-001.json",
    "index_path": "outputs/work/executions/example/index.json",
    "affected_task_ids": ["TASK-001"],
    "lock_status": "released",
}


class CorrectionContract(WorkContract):
    contract_id: ClassVar[str] = "work-correction/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "correction_id", "created_at", "target_attempt_id",
        "task_collection_sha256", "task_index_sha256", "task_item_sha256",
        "task_instructions_sha256", "execute_instructions_sha256",
        "field", "correct_value", "reason",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-correction/v1", "correction_id": "ATTEMPT-001-CORRECTION-001",
        "created_at": "2026-09-01T10:05+08:00", "target_attempt_id": "ATTEMPT-001",
        "task_collection_sha256": "a" * 64, "task_index_sha256": "b" * 64,
        "task_item_sha256": "c" * 64, "task_instructions_sha256": "d" * 64,
        "execute_instructions_sha256": "e" * 64, "field": "records[0].outcome",
        "correct_value": "passed", "reason": "Correct the recorded outcome.",
    }
    schema_: Literal["work-correction/v1"] = Field(alias="schema")
    correction_id: Any
    created_at: Any
    target_attempt_id: Any
    task_collection_sha256: Any
    task_index_sha256: Any
    task_item_sha256: Any
    task_instructions_sha256: Any
    execute_instructions_sha256: Any
    field: Any
    correct_value: Any
    reason: Any


__all__ = ["CorrectionContract", "CorrectionCreateContract", "CorrectionCreateRequestContract"]
