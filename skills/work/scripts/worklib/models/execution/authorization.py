from __future__ import annotations

from typing import ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from ..common.base import WorkContract
from ..task_collection import TaskCommandModel, TaskOperationModel, TaskValidationModel
from .deviation import ExecutionDeviationAction


REAPPROVAL_CONDITIONS = (
    "scope_expansion", "source_or_worktree_drift", "failure_divergence",
    "retry", "recovery", "unknown_result",
)


class AuthorizationNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class AttemptAuthorizationContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-authorization/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "commands", "validations", "modifiable_files",
        "working_directories", "external_operations", "allowed_deviations",
        "reapproval_conditions", "authorization_evidence",
    )

    schema_: Literal["work-attempt-authorization/v1"] = Field(alias="schema")
    task_id: str = Field(pattern=r"^TASK-[0-9]{3}$")
    commands: list[TaskCommandModel]
    validations: list[TaskValidationModel]
    modifiable_files: list[str]
    working_directories: list[str]
    external_operations: list[TaskOperationModel]
    allowed_deviations: list[ExecutionDeviationAction]
    reapproval_conditions: list[Literal[
        "scope_expansion", "source_or_worktree_drift", "failure_divergence",
        "retry", "recovery", "unknown_result",
    ]]
    authorization_evidence: str

    @model_validator(mode="after")
    def validate_closed_scope(self) -> Self:
        if tuple(self.reapproval_conditions) != REAPPROVAL_CONDITIONS:
            raise ValueError("Every fixed reapproval condition is required in canonical order.")
        if not self.authorization_evidence.strip():
            raise ValueError("authorization_evidence must be non-empty.")
        for values, label in (
            ([item.id for item in self.commands], "command"),
            ([item.id for item in self.validations], "validation"),
            ([item.id for item in self.external_operations], "external operation"),
            (self.modifiable_files, "modifiable file"),
            (self.working_directories, "working directory"),
        ):
            if len(values) != len(set(values)):
                raise ValueError(f"A {label} cannot be repeated.")
        return self


AttemptAuthorizationContract.contract_example = {
    "schema": "work-attempt-authorization/v1", "task_id": "TASK-001",
    "commands": [], "validations": [], "modifiable_files": [],
    "working_directories": [], "external_operations": [], "allowed_deviations": [],
    "reapproval_conditions": list(REAPPROVAL_CONDITIONS),
    "authorization_evidence": "User approved this exact Attempt scope.",
}
