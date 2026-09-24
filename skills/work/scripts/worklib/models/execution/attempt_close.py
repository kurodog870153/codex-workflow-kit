"""Attempt-close data models."""

from __future__ import annotations

from typing import ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from ..common.base import WorkContract
from ..common.errors import ExitCode, WorkError


class AttemptCloseRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-close-request/v1"
    contract_kind: ClassVar[Literal["semantic_request"]] = "semantic_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "final_type", "reason", "authorization_evidence",
    )
    schema_: Literal["work-attempt-close-request/v1"] = Field(alias="schema")
    status: Literal["completed", "stopped", "blocked"]
    final_type: str | None = None
    reason: str | None = None
    authorization_evidence: str | None = None

    @model_validator(mode="after")
    def validate_final_details(self) -> Self:
        present = [name for name in ("final_type", "reason") if getattr(self, name) is not None]
        if self.status == "completed" and present:
            raise WorkError(ExitCode.CONTRACT, "attempt_close_unexpected_final_details", "A completed Attempt cannot include final_type or reason.", {"fields": present})
        if self.status == "completed" and self.authorization_evidence is not None:
            raise WorkError(ExitCode.CONTRACT, "attempt_close_unexpected_authorization_evidence", "A completed Attempt reuses its manifest authorization.")
        if self.status != "completed":
            missing = [name for name in ("final_type", "reason") if getattr(self, name) is None]
            if missing:
                raise WorkError(ExitCode.CONTRACT, "attempt_close_missing_final_details", "A stopped or blocked Attempt requires final_type and reason.", {"missing": missing})
            if not self.authorization_evidence or not self.authorization_evidence.strip():
                raise WorkError(ExitCode.CONTRACT, "attempt_close_missing_authorization_evidence", "A stopped or blocked Attempt requires fresh authorization evidence.")
            if not self.final_type.strip() or not self.reason.strip():  # type: ignore[union-attr]
                raise WorkError(ExitCode.CONTRACT, "attempt_close_empty_final_detail", "final_type and reason must be non-empty strings.")
        return self


class PendingDeviationModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)

    deviation_id: str = Field(pattern=r"^DEVIATION-[0-9]{3}$")
    classification: Literal["task_only", "plan_and_task"]
    blocking: bool


class AttemptCloseContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-close/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "attempt_path", "index_path",
        "attempt_status", "task_status", "overall_status", "pending_deviations", "lock_status",
    )
    schema_: Literal["work-attempt-close/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str
    attempt_path: str
    index_path: str
    attempt_status: Literal["completed", "stopped", "blocked"]
    task_status: str
    overall_status: str
    pending_deviations: list[PendingDeviationModel]
    lock_status: Literal["released"]


AttemptCloseRequestContract.contract_example = {"schema": "work-attempt-close-request/v1", "status": "completed"}
AttemptCloseContract.contract_example = {
    "schema": "work-attempt-close/v1", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "attempt_status": "completed", "task_status": "completed",
    "overall_status": "completed", "pending_deviations": [], "lock_status": "released",
}


__all__ = ["AttemptCloseContract", "AttemptCloseRequestContract"]
