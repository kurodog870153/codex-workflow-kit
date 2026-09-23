from __future__ import annotations

from collections.abc import Mapping
from pathlib import PurePosixPath
from typing import Annotated, ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from ...protocol import SHA256_PATTERN
from ..common.base import WorkContract
from ..task_collection import (
    ExecutionDefaultsModel,
    TaskCommandModel,
    TaskOperationModel,
    TaskValidationModel,
)


NonEmptyText = Annotated[str, Field(min_length=1, pattern=r"\S")]
RecordId = Annotated[str, Field(pattern=r"^(?:CMD|OP|VAL)-[0-9]{3}(?:#[1-9][0-9]*)?$")]


class ExecutionDeviationNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)

    @staticmethod
    def validate_file_paths(paths: list[str]) -> None:
        if len(paths) != len(set(paths)):
            raise ValueError("A supplemental modifiable file cannot be repeated.")
        for path in paths:
            parsed = PurePosixPath(path)
            if (
                not path
                or "\\" in path
                or parsed.is_absolute()
                or path != parsed.as_posix()
                or ".." in parsed.parts
            ):
                raise ValueError("Supplemental file scope must use safe project-relative POSIX paths.")


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
    semantic_boundary_fields: ClassVar[tuple[str, ...]] = (
        "requirement_changed",
        "scope_changed",
        "deliverables_changed",
        "acceptance_criteria_changed",
        "safety_boundary_changed",
        "external_side_effect_boundary_changed",
    )

    summary: NonEmptyText
    requirement_changed: bool
    scope_changed: bool = False
    acceptance_criteria_changed: bool
    deliverables_changed: bool
    safety_boundary_changed: bool
    external_side_effect_boundary_changed: bool

    @classmethod
    def crosses_semantic_boundary(cls, impact: Mapping[str, object]) -> bool:
        return any(impact.get(field, False) for field in cls.semantic_boundary_fields)


class ExecutionDeviationDecisionModel(ExecutionDeviationNestedModel):
    outcome: Literal["approved", "rejected"]
    evidence: NonEmptyText


class ExecutionDeviationAuthorizationContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-authorization/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "preview_sha256", "action", "modifiable_files",
        "authorization_evidence",
    )

    schema_: Literal["work-execution-deviation-authorization/v1"] = Field(alias="schema")
    preview_sha256: Annotated[str, Field(pattern=SHA256_PATTERN)]
    action: ExecutionDeviationAction
    modifiable_files: list[str] = Field(default_factory=list)
    authorization_evidence: NonEmptyText

    @model_validator(mode="after")
    def validate_file_scope(self) -> Self:
        ExecutionDeviationNestedModel.validate_file_paths(self.modifiable_files)
        return self


class ExecutionDeviationProposalContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-proposal/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "anchor_record_id", "task_basis", "gap", "action",
        "modifiable_files", "impact", "side_effects",
    )

    schema_: Literal["work-execution-deviation-proposal/v1"] = Field(alias="schema")
    task_id: Annotated[str, Field(pattern=r"^TASK-[0-9]{3}$")]
    attempt_id: Annotated[str, Field(pattern=r"^ATTEMPT-[0-9]{3}$")]
    anchor_record_id: RecordId
    task_basis: Annotated[list[NonEmptyText], Field(min_length=1)]
    gap: NonEmptyText
    action: ExecutionDeviationAction
    modifiable_files: list[str] = Field(default_factory=list)
    impact: ExecutionDeviationImpactModel
    side_effects: list[NonEmptyText]

    @model_validator(mode="after")
    def validate_cross_field_invariants(self) -> Self:
        ExecutionDeviationNestedModel.validate_file_paths(self.modifiable_files)
        anchor = self.anchor_record_id
        base_anchor = anchor.split("#", 1)[0]
        if base_anchor not in self.task_basis:
            raise ValueError("task_basis must contain the anchor base record ID.")
        action = self.action
        if isinstance(action, (ReplaceCommandActionModel, SkipRecordActionModel)):
            if action.record_id != anchor:
                raise ValueError("The deviation action must target its anchor record.")
        elif isinstance(action, AddCommandActionModel):
            if action.after_record_id != anchor:
                raise ValueError("An added command must follow its anchor record.")
        elif isinstance(action, AdjustOperationActionModel):
            if action.operation.id != base_anchor:
                raise ValueError("An adjusted operation must preserve its anchor record ID.")
        return self


class ExecutionDeviationContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "deviation_id", "approved_preview_sha256", "proposal",
        "supplemental_authorization", "decision", "reconciliation_status",
    )

    schema_: Literal["work-execution-deviation/v1"] = Field(alias="schema")
    deviation_id: Annotated[str, Field(pattern=r"^DEVIATION-[0-9]{3}$")]
    approved_preview_sha256: Annotated[str, Field(pattern=SHA256_PATTERN)]
    proposal: ExecutionDeviationProposalContract
    supplemental_authorization: ExecutionDeviationAuthorizationContract
    decision: ExecutionDeviationDecisionModel
    reconciliation_status: Literal["pending", "incorporated", "declined", "not_needed"]

    @model_validator(mode="after")
    def validate_cross_field_invariants(self) -> ExecutionDeviationContract:
        authorization = self.supplemental_authorization
        if authorization.preview_sha256 != self.approved_preview_sha256:
            raise ValueError("Supplemental authorization must bind the approved preview fingerprint.")
        if authorization.action != self.proposal.action:
            raise ValueError("Supplemental action must match the approved proposal.")
        if authorization.authorization_evidence != self.decision.evidence:
            raise ValueError("The deviation decision must use its supplemental authorization evidence.")
        if self.decision.outcome == "rejected" and self.reconciliation_status != "not_needed":
            raise ValueError("A rejected deviation must use reconciliation_status not_needed.")
        if self.decision.outcome == "approved" and self.reconciliation_status == "not_needed":
            raise ValueError("An approved deviation must remain reconcilable.")
        if authorization.modifiable_files != self.proposal.modifiable_files:
            raise ValueError("Supplemental file scope must match the approved proposal.")
        return self


class ExecutionDeviationPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "proposal", "record_kind", "action_validation",
        "semantic_review", "classification", "blocking", "sources", "preview_sha256",
    )

    schema_: Literal["work-execution-deviation-preview/v1"] = Field(alias="schema")
    proposal: ExecutionDeviationProposalContract
    record_kind: Literal["command", "operation", "validation"]
    action_validation: Literal["passed"]
    semantic_review: Literal["required"]
    classification: Literal["task_only", "plan_and_task"]
    blocking: bool
    sources: dict[str, Annotated[str, Field(pattern=SHA256_PATTERN)]]
    preview_sha256: Annotated[str, Field(pattern=SHA256_PATTERN)]

    @model_validator(mode="after")
    def validate_cross_field_invariants(self) -> Self:
        blocking = self.proposal.impact.crosses_semantic_boundary(
            self.proposal.impact.model_dump()
        )
        classification = "plan_and_task" if blocking else "task_only"
        if self.classification != classification or self.blocking != blocking:
            raise ValueError("Preview classification and blocking must match proposal impact.")
        return self


class ExecutionDeviationRecordContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-deviation-record/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "deviation_id", "attempt_path",
        "classification", "blocking", "record_status", "lock_status",
    )

    schema_: Literal["work-execution-deviation-record/v1"] = Field(alias="schema")
    task_id: Annotated[str, Field(pattern=r"^TASK-[0-9]{3}$")]
    attempt_id: Annotated[str, Field(pattern=r"^ATTEMPT-[0-9]{3}$")]
    deviation_id: Annotated[str, Field(pattern=r"^DEVIATION-[0-9]{3}$")]
    attempt_path: NonEmptyText
    classification: Literal["task_only", "plan_and_task"]
    blocking: bool
    record_status: Literal["recorded"]
    lock_status: Literal["record_reserved"]

    @model_validator(mode="after")
    def validate_cross_field_invariants(self) -> Self:
        if self.blocking != (self.classification == "plan_and_task"):
            raise ValueError("Record blocking must match its reconciliation classification.")
        expected_suffix = f"/{self.task_id}/{self.attempt_id}/attempt.json"
        if not self.attempt_path.endswith(expected_suffix):
            raise ValueError("Record Attempt path must match its TASK and Attempt identities.")
        return self


_IMPACT_EXAMPLE = {
    "summary": "Use the absolute executable path without changing command effects.",
    "requirement_changed": False,
    "scope_changed": False,
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
    "modifiable_files": [],
    "impact": _IMPACT_EXAMPLE,
    "side_effects": ["Runs the existing validation command."],
}
ExecutionDeviationContract.contract_example = {
    "schema": "work-execution-deviation/v1",
    "deviation_id": "DEVIATION-001",
    "approved_preview_sha256": "c" * 64,
    "proposal": ExecutionDeviationProposalContract.contract_example,
    "supplemental_authorization": {
        "schema": "work-execution-deviation-authorization/v1",
        "preview_sha256": "c" * 64,
        "action": ExecutionDeviationProposalContract.contract_example["action"],
        "modifiable_files": [],
        "authorization_evidence": "User approved this exact deviation preview.",
    },
    "decision": {
        "outcome": "approved",
        "evidence": "User approved this exact deviation preview.",
    },
    "reconciliation_status": "pending",
}
ExecutionDeviationAuthorizationContract.contract_example = {
    "schema": "work-execution-deviation-authorization/v1",
    "preview_sha256": "c" * 64,
    "action": ExecutionDeviationProposalContract.contract_example["action"],
    "modifiable_files": [],
    "authorization_evidence": "User approved this exact deviation preview.",
}
ExecutionDeviationPreviewContract.contract_example = {
    "schema": "work-execution-deviation-preview/v1",
    "proposal": ExecutionDeviationProposalContract.contract_example,
    "record_kind": "command",
    "action_validation": "passed",
    "semantic_review": "required",
    "classification": "task_only",
    "blocking": False,
    "sources": {"outputs/work/tasks/example/index.json": "a" * 64},
    "preview_sha256": "b" * 64,
}
ExecutionDeviationRecordContract.contract_example = {
    "schema": "work-execution-deviation-record/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "deviation_id": "DEVIATION-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "classification": "task_only",
    "blocking": False,
    "record_status": "recorded",
    "lock_status": "record_reserved",
}
