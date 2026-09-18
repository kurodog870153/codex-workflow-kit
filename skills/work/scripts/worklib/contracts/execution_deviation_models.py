from __future__ import annotations

from typing import Annotated, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .base import WorkContract
from .task_collection_models import (
    ExecutionDefaultsModel,
    TaskCommandModel,
    TaskOperationModel,
    TaskValidationModel,
)


NonEmptyText = Annotated[str, Field(min_length=1, pattern=r"\S")]
RecordId = Annotated[str, Field(pattern=r"^(?:CMD|OP|VAL)-[0-9]{3}(?:#[1-9][0-9]*)?$")]


class ExecutionDeviationNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class DeviationCommandModel(ExecutionDeviationNestedModel):
    mode: Literal["argv", "shell"]
    argv: list[NonEmptyText] | None = None
    script: NonEmptyText | None = None
    execution: ExecutionDefaultsModel | None = None

    @model_validator(mode="after")
    def validate_command_value(self) -> DeviationCommandModel:
        if self.mode == "argv" and (not self.argv or self.script is not None):
            raise ValueError("argv mode requires only a non-empty argv array.")
        if self.mode == "shell" and (self.argv is not None or self.script is None):
            raise ValueError("shell mode requires only a non-empty script.")
        return self


class ReplaceCommandActionModel(ExecutionDeviationNestedModel):
    kind: Literal["replace_command"]
    record_id: RecordId
    replacement: DeviationCommandModel


class AddCommandActionModel(ExecutionDeviationNestedModel):
    kind: Literal["add_command"]
    after_record_id: RecordId
    command: TaskCommandModel


class AddValidationActionModel(ExecutionDeviationNestedModel):
    kind: Literal["add_validation"]
    validation: TaskValidationModel


class SkipRecordActionModel(ExecutionDeviationNestedModel):
    kind: Literal["skip_record"]
    record_id: RecordId
    reason: NonEmptyText


class AdjustOperationActionModel(ExecutionDeviationNestedModel):
    kind: Literal["adjust_operation"]
    operation: TaskOperationModel


ExecutionDeviationAction = Annotated[
    ReplaceCommandActionModel
    | AddCommandActionModel
    | AddValidationActionModel
    | SkipRecordActionModel
    | AdjustOperationActionModel,
    Field(discriminator="kind"),
]


class ExecutionDeviationImpactModel(ExecutionDeviationNestedModel):
    summary: NonEmptyText
    requirement_changed: Literal[False]
    acceptance_criteria_changed: Literal[False]
    deliverables_changed: Literal[False]
    safety_boundary_changed: Literal[False]
    external_side_effect_boundary_changed: Literal[False]


class ExecutionDeviationDecisionModel(ExecutionDeviationNestedModel):
    outcome: Literal["approved", "rejected"]
    evidence: NonEmptyText


class ExecutionDeviationProposalContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-proposal/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "anchor_record_id", "task_basis", "gap", "action",
        "impact", "side_effects",
    )

    schema_: Literal["work-execution-deviation-proposal/v1"] = Field(alias="schema")
    task_id: Annotated[str, Field(pattern=r"^TASK-[0-9]{3}$")]
    attempt_id: Annotated[str, Field(pattern=r"^ATTEMPT-[0-9]{3}$")]
    anchor_record_id: RecordId
    task_basis: Annotated[list[NonEmptyText], Field(min_length=1)]
    gap: NonEmptyText
    action: ExecutionDeviationAction
    impact: ExecutionDeviationImpactModel
    side_effects: list[NonEmptyText]


class ExecutionDeviationContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "deviation_id", "approved_preview_sha256", "proposal", "decision",
        "reconciliation_status",
    )

    schema_: Literal["work-execution-deviation/v1"] = Field(alias="schema")
    deviation_id: Annotated[str, Field(pattern=r"^DEVIATION-[0-9]{3}$")]
    approved_preview_sha256: Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]
    proposal: ExecutionDeviationProposalContract
    decision: ExecutionDeviationDecisionModel
    reconciliation_status: Literal["pending", "incorporated", "declined", "not_needed"]

    @model_validator(mode="after")
    def validate_rejected_reconciliation(self) -> ExecutionDeviationContract:
        if self.decision.outcome == "rejected" and self.reconciliation_status != "not_needed":
            raise ValueError("A rejected deviation must use reconciliation_status not_needed.")
        return self


class ExecutionDeviationPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "proposal", "record_kind", "action_validation",
        "semantic_review", "sources", "preview_sha256",
    )

    schema_: Literal["work-execution-deviation-preview/v1"] = Field(alias="schema")
    proposal: ExecutionDeviationProposalContract
    record_kind: Literal["command", "operation", "validation"]
    action_validation: Literal["passed"]
    semantic_review: Literal["required"]
    sources: dict[str, Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]]
    preview_sha256: Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]


class ExecutionDeviationRecordContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-record/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "deviation_id", "attempt_path",
        "record_status", "lock_status",
    )

    schema_: Literal["work-execution-deviation-record/v1"] = Field(alias="schema")
    task_id: Annotated[str, Field(pattern=r"^TASK-[0-9]{3}$")]
    attempt_id: Annotated[str, Field(pattern=r"^ATTEMPT-[0-9]{3}$")]
    deviation_id: Annotated[str, Field(pattern=r"^DEVIATION-[0-9]{3}$")]
    attempt_path: NonEmptyText
    record_status: Literal["recorded"]
    lock_status: Literal["record_reserved"]


_IMPACT_EXAMPLE = {
    "summary": "Use the absolute executable path without changing command effects.",
    "requirement_changed": False,
    "acceptance_criteria_changed": False,
    "deliverables_changed": False,
    "safety_boundary_changed": False,
    "external_side_effect_boundary_changed": False,
}
ExecutionDeviationProposalContract.contract_example = {
    "schema": "work-execution-deviation-proposal/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "anchor_record_id": "CMD-001",
    "task_basis": ["CMD-001", "STEP-001"],
    "gap": "The executable is not available through PATH.",
    "action": {
        "kind": "replace_command",
        "record_id": "CMD-001",
        "replacement": {"mode": "argv", "argv": ["C:/tools/tool.cmd", "test"]},
    },
    "impact": _IMPACT_EXAMPLE,
    "side_effects": ["Runs the existing validation command."],
}
ExecutionDeviationContract.contract_example = {
    "schema": "work-execution-deviation/v1",
    "deviation_id": "DEVIATION-001",
    "approved_preview_sha256": "c" * 64,
    "proposal": ExecutionDeviationProposalContract.contract_example,
    "decision": {
        "outcome": "approved",
        "evidence": "User approved the reviewed replacement command.",
    },
    "reconciliation_status": "pending",
}
ExecutionDeviationPreviewContract.contract_example = {
    "schema": "work-execution-deviation-preview/v1",
    "proposal": ExecutionDeviationProposalContract.contract_example,
    "record_kind": "command",
    "action_validation": "passed",
    "semantic_review": "required",
    "sources": {"outputs/work/tasks/example/index.json": "a" * 64},
    "preview_sha256": "b" * 64,
}
ExecutionDeviationRecordContract.contract_example = {
    "schema": "work-execution-deviation-record/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "deviation_id": "DEVIATION-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "record_status": "recorded",
    "lock_status": "record_reserved",
}
