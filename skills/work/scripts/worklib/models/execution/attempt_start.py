"""Attempt-start data models."""

from __future__ import annotations

from typing import ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from ...protocol import ATTEMPT_ID_PATTERN, SHA256_PATTERN
from ..common.base import WorkContract
from ..common.errors import ExitCode, WorkError
from .authorization import AttemptAuthorizationContract


ATTEMPT_PATTERN = ATTEMPT_ID_PATTERN
RECORD_PATTERN = r"^(?:STEP|CMD|OP|VAL)-\d{3}(?:#\d+)?$"


class AttemptStartNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class CarriedRecordModel(AttemptStartNestedModel):
    record_id: str = Field(pattern=RECORD_PATTERN)
    evidence: str

    @field_validator("evidence")
    @classmethod
    def nonempty_evidence(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("Evidence must be non-empty.")
        return value


class AttemptContinuationModel(AttemptStartNestedModel):
    source_attempt_id: str = Field(pattern=ATTEMPT_PATTERN)
    carried_records: list[CarriedRecordModel]

    @model_validator(mode="after")
    def unique_records(self) -> Self:
        seen: set[str] = set()
        for record in self.carried_records:
            if record.record_id in seen:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "attempt_start_duplicate_carried_record",
                    "A carried record ID cannot be repeated.",
                    {"record_id": record.record_id},
                )
            seen.add(record.record_id)
        return self


class AttemptStartRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-start-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "worktree_snapshot_sha256", "authorization", "continuation",
    )
    schema_: Literal["work-attempt-start-request/v1"] = Field(alias="schema")
    worktree_snapshot_sha256: str = Field(pattern=SHA256_PATTERN)
    authorization: AttemptAuthorizationContract
    continuation: AttemptContinuationModel | None = None


class AttemptStartContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-start/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "attempt_path", "index_path", "status", "lock_status",
    )
    schema_: Literal["work-attempt-start/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str = Field(pattern=ATTEMPT_PATTERN)
    attempt_path: str
    index_path: str
    status: Literal["started"]
    lock_status: Literal["held"]


class AttemptStartRecoveryContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-start-recovery/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = AttemptStartContract.canonical_order
    schema_: Literal["work-attempt-start-recovery/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str = Field(pattern=ATTEMPT_PATTERN)
    attempt_path: str
    index_path: str
    status: Literal["recovered"]
    lock_status: Literal["held"]


AttemptStartRequestContract.contract_example = {
    "schema": "work-attempt-start-request/v1", "worktree_snapshot_sha256": "0" * 64,
    "authorization": AttemptAuthorizationContract.contract_example,
}
AttemptStartContract.contract_example = {
    "schema": "work-attempt-start/v1", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json", "status": "started", "lock_status": "held",
}
AttemptStartRecoveryContract.contract_example = {
    **AttemptStartContract.contract_example,
    "schema": "work-attempt-start-recovery/v1", "status": "recovered",
}


__all__ = [
    "AttemptContinuationModel", "AttemptStartContract", "AttemptStartNestedModel",
    "AttemptStartRecoveryContract", "AttemptStartRequestContract", "CarriedRecordModel",
]
