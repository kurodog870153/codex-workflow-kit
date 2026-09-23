"""Public contracts for reviewed TASK collection repair."""
from __future__ import annotations

from typing import Annotated, Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError, model_validator

from ...protocol import SHA256_PATTERN
from ..common.base import WorkContract
from ..common.semantic import SemanticEvidencePolicy
from .contracts import TaskArtifactsModel, TaskIndexContract, TaskItemContract
from ..common.errors import ExitCode, WorkError


class TaskRepairNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class TaskRepairDecisionModel(TaskRepairNestedModel):
    location: str = Field(min_length=1)
    decision: str = Field(min_length=1)


class TaskRepairSemanticEditModel(TaskRepairNestedModel):
    field: str = Field(min_length=1, pattern=r"^[^/~]+$")
    task_id: str | None = None
    after: str | None = None
    semantic_after: Any = None
    remove: bool = False

    @model_validator(mode="after")
    def validate_evidence(self) -> "TaskRepairSemanticEditModel":
        simple = {"title", "goal"} if self.task_id is not None else {"title", "summary"}
        semantic = ({"traceability", "dependencies", "inputs", "decisions", "files", "risks",
                     "steps", "commands", "operations", "validations"}
                    if self.task_id is not None else {"decisions", "execution_defaults"})
        if self.field not in simple | semantic:
            raise ValueError("This repair field has no semantic operation.")
        supplied = self.model_fields_set
        if self.remove:
            if self.field in simple or supplied & {"after", "semantic_after"}:
                raise ValueError("Only an optional semantic field can be removed.")
        elif self.field in simple:
            if "after" not in supplied or "semantic_after" in supplied:
                raise ValueError("Simple repair fields require after.")
        elif "semantic_after" not in supplied or "after" in supplied or self.semantic_after is None:
            raise ValueError("Nested repair fields require semantic_after.")
        else:
            SemanticEvidencePolicy.reject_formal_data(self.semantic_after)
        return self


class TaskRepairMissingSemanticTaskModel(TaskRepairNestedModel):
    task_position: int = Field(ge=1)
    title: str = Field(min_length=1)
    goal: str = Field(min_length=1)
    skill_id: str | None = None
    selected_paths: list[str]
    references: list[str]
    dependency_positions: list[int] = Field(default_factory=list)
    candidate: dict[str, Any]

    @model_validator(mode="after")
    def semantic_candidate(self) -> "TaskRepairMissingSemanticTaskModel":
        SemanticEvidencePolicy.reject_formal_data(self.candidate)
        return self


class TaskRepairPrepareRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-repair-prepare-request/v1"
    contract_kind: ClassVar[Literal["semantic_request"]] = "semantic_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "stage", "requirement_id", "decisions", "edits", "missing_task",
    )
    schema_: Literal["work-task-repair-prepare-request/v1"] = Field(alias="schema")
    stage: Literal["format", "complete"]
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    decisions: list[TaskRepairDecisionModel] = Field(min_length=1)
    edits: list[TaskRepairSemanticEditModel] | None = None
    missing_task: TaskRepairMissingSemanticTaskModel | None = None

    @model_validator(mode="after")
    def validate_choice(self) -> "TaskRepairPrepareRequestContract":
        if self.edits is not None and self.missing_task is not None:
            raise ValueError("Use edits or one missing semantic TASK, not both.")
        return self

    @classmethod
    def _work_error(cls, error: ValidationError) -> WorkError:
        first = error.errors(include_url=False, include_context=False, include_input=False)[0]
        location = tuple(first["loc"])
        if location == ("schema",) and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_prepare_schema",
                             "Use work-task-repair-prepare-request/v1.")
        if location == ("decisions",) and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_decisions",
                             "TASK collection repair requires explicit reviewed decisions.")
        return super()._work_error(error)


class TaskRepairRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-repair-request/v1"
    contract_kind: ClassVar[Literal["generated_request"]] = "generated_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "stage", "requirement_id", "artifacts", "expected", "decisions", "task_index", "task_items",
    )
    schema_: Literal["work-task-repair-request/v1"] = Field(alias="schema")
    stage: Literal["format", "complete"]
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    artifacts: TaskArtifactsModel
    expected: dict[str, Annotated[str, Field(pattern=SHA256_PATTERN)] | None]
    decisions: list[TaskRepairDecisionModel] = Field(min_length=1)
    task_index: TaskIndexContract
    task_items: dict[str, TaskItemContract] = Field(min_length=1)

    @classmethod
    def _work_error(cls, error: ValidationError) -> WorkError:
        first = error.errors(include_url=False, include_context=False, include_input=False)[0]
        location = tuple(first["loc"])
        if location and location[0] == "expected" and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_expected",
                             "Expected evidence must map paths to SHA-256 fingerprints or null.")
        if location == ("schema",) and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_schema",
                             "Use work-task-repair-request/v1.")
        if location == ("decisions",) and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_decisions",
                             "TASK collection repair requires explicit reviewed decisions.")
        if location in {("task_index",), ("task_items",)} and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_candidate",
                             "TASK collection repair requires an explicit complete index and TASK item map.")
        return super()._work_error(error)


class TaskRepairContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-repair/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "stage", "approved_sha256", "artifacts", "decisions",
        "affected_task_ids", "changed_paths", "task_diagnostics", "file_readiness",
        "publication_status",
    )
    schema_: Literal["work-task-repair/v1"] = Field(alias="schema")
    status: Literal["preview", "repaired", "recovered", "already_completed"]
    stage: Literal["format", "complete"]
    approved_sha256: str = Field(pattern=SHA256_PATTERN)
    artifacts: TaskArtifactsModel
    decisions: list[TaskRepairDecisionModel]
    affected_task_ids: list[str]
    changed_paths: list[str]
    task_diagnostics: dict[str, Any]
    file_readiness: Literal["requires_execute_preflight"]
    publication_status: Literal["published", "already_published"] | None = None


class TaskRepairPrepareContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-repair-prepare/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "request", "preview", "output_file")
    field_references: ClassVar[dict[str, str]] = {
        "request": "work-task-repair-request/v1", "preview": "work-task-repair/v1",
    }
    schema_: Literal["work-task-repair-prepare/v1"] = Field(alias="schema")
    request: TaskRepairRequestContract
    preview: TaskRepairContract
    output_file: str | None


_ARTIFACTS = {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json",
              "execution": "outputs/work/executions/example"}
_DECISIONS = [{"location": "/", "decision": "Use the reviewed collection."}]
TaskRepairPrepareRequestContract.contract_example = {
    "schema": "work-task-repair-prepare-request/v1", "stage": "complete", "requirement_id": "example",
    "decisions": _DECISIONS,
}
TaskRepairRequestContract.contract_example = {
    **TaskRepairPrepareRequestContract.contract_example, "schema": "work-task-repair-request/v1",
    "artifacts": _ARTIFACTS,
    "expected": {"outputs/work/tasks/example/index.json": "0" * 64},
    "task_index": TaskIndexContract.contract_example, "task_items": {"TASK-001": TaskItemContract.contract_example},
}
TaskRepairContract.contract_example = {
    "schema": "work-task-repair/v1", "status": "preview", "stage": "complete",
    "approved_sha256": "0" * 64, "artifacts": _ARTIFACTS, "decisions": _DECISIONS,
    "affected_task_ids": ["TASK-001"], "changed_paths": ["outputs/work/tasks/example/index.json"],
    "task_diagnostics": {}, "file_readiness": "requires_execute_preflight",
}
TaskRepairPrepareContract.contract_example = {
    "schema": "work-task-repair-prepare/v1", "request": TaskRepairRequestContract.contract_example,
    "preview": TaskRepairContract.contract_example, "output_file": None,
}
