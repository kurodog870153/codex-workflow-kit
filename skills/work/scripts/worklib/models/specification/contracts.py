"""Public contracts for reviewed specification preparation."""
from __future__ import annotations

from typing import Annotated, Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError, model_validator

from ...protocol import SHA256_PATTERN
from ..common.base import WorkContract
from ..common.semantic import SemanticEvidencePolicy
from ..plan import PlanContract
from .transaction import SpecTransactionContract
from ..task_collection import TaskArtifactsModel, TaskIndexContract, TaskItemContract
from ..common.errors import ExitCode, WorkError


class SpecificationTargetModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)

    artifact: Literal["plan", "task_index", "task_item"]
    task_id: str | None = None


_SIMPLE_FIELDS = {
    "plan": {"title", "summary"},
    "task_index": {"title", "summary"},
    "task_item": {"title", "goal"},
}
_SEMANTIC_FIELDS = {
    "plan": {"goals", "scope", "constraints", "dependencies", "risks", "milestones", "deliverables", "acceptance_criteria", "decisions"},
    "task_index": {"decisions", "execution_defaults"},
    "task_item": {"traceability", "dependencies", "inputs", "decisions", "files", "risks", "steps", "commands", "operations", "validations"},
}
class SpecificationSemanticTaskModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)

    title: str = Field(min_length=1)
    goal: str = Field(min_length=1)
    skill_id: str | None
    selected_paths: list[str]
    references: list[str]
    dependency_positions: list[int] = Field(default_factory=list)
    candidate: dict[str, Any]

    @model_validator(mode="after")
    def semantic_candidate(self) -> "SpecificationSemanticTaskModel":
        SemanticEvidencePolicy.reject_formal_data(self.candidate)
        return self


class SpecificationSemanticEditModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)

    target: SpecificationTargetModel | None = None
    field: str | None = None
    after: str | None = None
    semantic_after: Any = None
    operation: Literal["add_task", "remove_task"] | None = None
    task: SpecificationSemanticTaskModel | None = None
    task_position: int | None = None

    @model_validator(mode="after")
    def validate_semantic_operation(self) -> "SpecificationSemanticEditModel":
        supplied = self.model_fields_set
        if self.operation == "add_task":
            if supplied != {"operation", "task"} or self.task is None:
                raise ValueError("add_task requires only a semantic task.")
            return self
        if self.operation == "remove_task":
            if supplied != {"operation", "task_position"} or self.task_position is None or self.task_position < 1:
                raise ValueError("remove_task requires one-based task_position.")
            return self
        if self.operation is not None or self.target is None or self.field is None or supplied & {"task", "task_position"}:
            raise ValueError("A field edit requires a target and field.")
        artifact = self.target.artifact
        if (artifact == "task_item") != (self.target.task_id is not None):
            raise ValueError("Only a task_item field edit requires task_id.")
        if self.field in _SIMPLE_FIELDS[artifact]:
            if "after" not in supplied or "semantic_after" in supplied:
                raise ValueError("Simple fields require after.")
        elif self.field in _SEMANTIC_FIELDS[artifact]:
            if "semantic_after" not in supplied or "after" in supplied:
                raise ValueError("Machine-bearing fields require semantic_after.")
            SemanticEvidencePolicy.reject_formal_data(self.semantic_after)
        else:
            raise ValueError("This specification field has no semantic operation.")
        return self


class SpecificationPrepareRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-prepare-request/v1"
    contract_kind: ClassVar[Literal["semantic_request"]] = "semantic_request"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "requirement_id", "reason", "edits")
    field_constraints: ClassVar[dict[str, dict[str, Any]]] = {
        "edits": {"min_length": 1},
    }

    schema_: Literal["work-spec-prepare-request/v1"] = Field(alias="schema")
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    reason: str
    edits: list[SpecificationSemanticEditModel] = Field(min_length=1)

    @classmethod
    def _work_error(cls, error: ValidationError) -> WorkError:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first["loc"])
        # Keep the established CLI error categories at the new typed boundary.
        mapped = None
        if location == ("schema",) and first["type"] != "missing":
            mapped = ("spec_prepare_schema", "Use work-spec-prepare-request/v1.")
        elif location == ("edits",) and first["type"] != "missing":
            mapped = ("spec_prepare_edits", "Supply non-empty collection edits.")
        elif len(location) == 4 and location[0] == "edits" and first["type"] != "missing":
            if location[-1] == "artifact":
                mapped = ("spec_prepare_artifact", "Use plan, task_index or task_item.")
        if mapped is not None:
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, *mapped)
        if location == ("requirement_id",) and first["type"] != "missing":
            return WorkError(ExitCode.CONTRACT, "empty_text_value", "A non-empty string is required.",
                             {"location": "requirement_id"})
        result = super()._work_error(error)
        if result.details.get("location") == "contract":
            result.details["location"] = "spec_prepare"
        if first["type"] == "model_type":
            path = "spec_prepare" if not location else f"edits[{location[1]}]"
            return WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.",
                             {"location": path})
        return result


SpecificationPrepareRequestContract.contract_example = {
    "schema": "work-spec-prepare-request/v1",
    "requirement_id": "example",
    "reason": "Confirmed goal revision",
    "edits": [{"target": {"artifact": "task_item", "task_id": "TASK-001"},
               "field": "goal", "after": "Reviewed goal"}],
}


class SpecificationExpectedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)

    plan_sha256: Annotated[str, Field(pattern=SHA256_PATTERN)]
    task_index_sha256: Annotated[str, Field(pattern=SHA256_PATTERN)]
    execution_index_sha256: Annotated[str, Field(pattern=SHA256_PATTERN)]
    task_item_sha256: dict[str, Annotated[str, Field(pattern=SHA256_PATTERN)]]


class SpecificationUpdateRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-update-request/v1"
    contract_kind: ClassVar[Literal["generated_request"]] = "generated_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "reason", "expected", "plan", "task_index", "task_items",
    )
    field_references: ClassVar[dict[str, str]] = {
        "plan": "work-plan/v1", "task_index": "work-task-index/v1",
        "task_items": "work-task-item/v1",
    }

    schema_: Literal["work-spec-update-request/v1"] = Field(alias="schema")
    reason: str
    expected: SpecificationExpectedModel
    plan: PlanContract
    task_index: TaskIndexContract
    task_items: dict[str, TaskItemContract]

    @classmethod
    def _work_error(cls, error: ValidationError) -> WorkError:
        first = error.errors(include_url=False, include_context=False, include_input=False)[0]
        location = tuple(first["loc"])
        if location == ("schema",) and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "spec_update_schema",
                             "Use work-spec-update-request/v1.")
        if location == ("plan",) and first["type"] in {"missing", "model_type"}:
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "spec_update_candidate",
                             "A complete Plan is required.")
        if location and location[0] == "expected" and location != ("expected",):
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "spec_update_source_changed",
                             "The reviewed source fingerprints changed.")
        result = super()._work_error(error)
        if result.details.get("location") == "contract":
            result.details["location"] = "spec__update_collection"
        return result


SpecificationUpdateRequestContract.contract_example = {
    "schema": "work-spec-update-request/v1", "reason": "Reviewed collection revision",
    "expected": {
        "plan_sha256": "0" * 64, "task_index_sha256": "0" * 64,
        "execution_index_sha256": "0" * 64, "task_item_sha256": {"TASK-001": "0" * 64},
    },
    "plan": PlanContract.contract_example,
    "task_index": TaskIndexContract.contract_example,
    "task_items": {"TASK-001": TaskItemContract.contract_example},
}


class SpecificationVerificationRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-verification-request/v1"
    contract_kind: ClassVar[Literal["generated_request"]] = "generated_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "artifacts", "record_id",
    )
    field_constraints: ClassVar[dict[str, dict[str, Any]]] = {
        "record_id": {"pattern": r"^SPEC-UPDATE-[0-9A-F]{12}$"},
    }
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-spec-verification-request/v1", "requirement_id": "example",
        "artifacts": {"plan": "outputs/work/plans/example.json",
                      "task": "outputs/work/tasks/example/index.json",
                      "execution": "outputs/work/executions/example"},
        "record_id": SpecTransactionContract.contract_example["transaction_id"],
    }

    schema_: Literal["work-spec-verification-request/v1"] = Field(alias="schema")
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    artifacts: TaskArtifactsModel
    record_id: str = Field(pattern=r"^SPEC-UPDATE-[0-9A-F]{12}$")


class SpecificationNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class SpecificationCandidateModel(SpecificationNestedModel):
    plan: PlanContract
    task_index: TaskIndexContract
    task_items: dict[str, TaskItemContract]


class SpecificationPublishStepModel(SpecificationNestedModel):
    command: Literal["task spec-update"]
    input: Literal["same_request"]
    approved_sha256: str = Field(pattern=SHA256_PATTERN)


class SpecificationVerifyStepModel(SpecificationNestedModel):
    command: Literal["task spec-verify"]
    input: Literal["verification_request"]


class SpecificationUpdateContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-update/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "requirement_id", "record_id", "approved_sha256",
        "affected_task_ids", "changed_fields", "artifacts", "candidate", "transaction",
        "file_readiness", "next_step", "publication_status", "verification_request",
    )
    field_references: ClassVar[dict[str, str]] = {
        "transaction": "work-spec-transaction/v1",
        "verification_request": "work-spec-verification-request/v1",
    }
    schema_: Literal["work-spec-update/v1"] = Field(alias="schema")
    status: Literal["valid", "updated", "recovered"]
    requirement_id: str
    record_id: str = Field(pattern=r"^SPEC-UPDATE-[0-9A-F]{12}$")
    approved_sha256: str = Field(pattern=SHA256_PATTERN)
    affected_task_ids: list[str]
    changed_fields: list[str] | None = None
    artifacts: TaskArtifactsModel
    candidate: SpecificationCandidateModel | None = None
    transaction: SpecTransactionContract | None = None
    file_readiness: Literal["requires_execute_preflight"] | None = None
    next_step: SpecificationPublishStepModel | SpecificationVerifyStepModel | None = None
    publication_status: Literal["published", "already_published"] | None = None
    verification_request: SpecificationVerificationRequestContract | None = None

    @model_validator(mode="after")
    def validate_status_fields(self) -> "SpecificationUpdateContract":
        if self.status in {"valid", "updated"}:
            if self.candidate is None or self.changed_fields is None or self.file_readiness is None:
                raise ValueError("Preview and update responses require complete candidate evidence.")
        if self.status == "valid":
            if self.transaction is None or self.publication_status is not None or self.verification_request is not None:
                raise ValueError("A preview requires a transaction and cannot claim publication.")
            if self.next_step is not None and not isinstance(self.next_step, SpecificationPublishStepModel):
                raise ValueError("A preview may only point to publication.")
        else:
            if self.transaction is not None or self.publication_status is None or self.verification_request is None:
                raise ValueError("Publication requires its status and verification request, without a transaction payload.")
            if not isinstance(self.next_step, SpecificationVerifyStepModel):
                raise ValueError("Publication must point to verification.")
        return self


class SpecificationTransportModel(SpecificationNestedModel):
    request_field: Literal["request"]
    request_schema: Literal["work-spec-update-request/v1"]
    output_file: str | None


class SpecificationValidateStepModel(SpecificationNestedModel):
    command: Literal["task spec-validate"]
    input: Literal["request"]


class SpecificationPrepareContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-prepare/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "request", "preview", "output_file", "transport", "next_step",
    )
    field_references: ClassVar[dict[str, str]] = {
        "request": "work-spec-update-request/v1", "preview": "work-spec-update/v1",
    }
    schema_: Literal["work-spec-prepare/v1"] = Field(alias="schema")
    request: SpecificationUpdateRequestContract
    preview: SpecificationUpdateContract
    output_file: str | None
    transport: SpecificationTransportModel
    next_step: SpecificationValidateStepModel

    @model_validator(mode="after")
    def validate_preparation(self) -> "SpecificationPrepareContract":
        if self.preview.status != "valid" or self.output_file != self.transport.output_file:
            raise ValueError("Preparation requires a valid preview and matching output transport.")
        return self


class SpecificationVerificationContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-verification/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "verified", "record_id", "requirement_id", "artifacts",
        "task_collection_sha256", "journal_sha256", "verification_scope",
        "execution_authorized", "next_step",
    )
    schema_: Literal["work-spec-verification/v1"] = Field(alias="schema")
    status: Literal["verified"]
    verified: Literal[True]
    record_id: str = Field(pattern=r"^SPEC-UPDATE-[0-9A-F]{12}$")
    requirement_id: str
    artifacts: TaskArtifactsModel
    task_collection_sha256: str = Field(pattern=SHA256_PATTERN)
    journal_sha256: str = Field(pattern=SHA256_PATTERN)
    verification_scope: Literal["exact_specification_result"]
    execution_authorized: Literal[False]
    next_step: Literal["normal_execute_preflight"]


SpecificationUpdateContract.contract_example = {
    "schema": "work-spec-update/v1", "status": "valid", "requirement_id": "example",
    "record_id": SpecTransactionContract.contract_example["transaction_id"], "approved_sha256": SpecTransactionContract.contract_example["approval_sha256"],
    "affected_task_ids": ["TASK-001"], "changed_fields": ["/task_items/TASK-001/goal"],
    "artifacts": SpecificationVerificationRequestContract.contract_example["artifacts"],
    "candidate": {key: SpecificationUpdateRequestContract.contract_example[key]
                  for key in ("plan", "task_index", "task_items")},
    "transaction": SpecTransactionContract.contract_example,
    "file_readiness": "requires_execute_preflight",
}
SpecificationPrepareContract.contract_example = {
    "schema": "work-spec-prepare/v1", "request": SpecificationUpdateRequestContract.contract_example,
    "preview": SpecificationUpdateContract.contract_example, "output_file": None,
    "transport": {"request_field": "request", "request_schema": "work-spec-update-request/v1", "output_file": None},
    "next_step": {"command": "task spec-validate", "input": "request"},
}
SpecificationVerificationContract.contract_example = {
    "schema": "work-spec-verification/v1", "status": "verified", "verified": True,
    "record_id": SpecTransactionContract.contract_example["transaction_id"], "requirement_id": "example",
    "artifacts": SpecificationVerificationRequestContract.contract_example["artifacts"],
    "task_collection_sha256": "0" * 64, "journal_sha256": "0" * 64,
    "verification_scope": "exact_specification_result", "execution_authorized": False,
    "next_step": "normal_execute_preflight",
}

__all__ = [
    "SpecificationTargetModel", "SpecificationSemanticEditModel",
    "SpecificationPrepareRequestContract",
    "SpecificationExpectedModel",
    "SpecificationUpdateRequestContract",
    "SpecificationVerificationRequestContract",
    "SpecificationNestedModel",
    "SpecificationCandidateModel",
    "SpecificationPublishStepModel",
    "SpecificationVerifyStepModel",
    "SpecificationUpdateContract",
    "SpecificationTransportModel",
    "SpecificationValidateStepModel",
    "SpecificationPrepareContract",
    "SpecificationVerificationContract",
]
