"""Public contracts for reviewed TASK collection repair."""
from __future__ import annotations

from typing import Annotated, Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from ..models.common.base import WorkContract
from .task_collection_models import TaskArtifactsModel, TaskIndexContract, TaskItemContract
from ..models.common.errors import ExitCode, WorkError


class TaskRepairNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class TaskRepairDecisionModel(TaskRepairNestedModel):
    location: str = Field(min_length=1)
    decision: str = Field(min_length=1)


class TaskRepairPrepareRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-repair-prepare-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "stage", "requirement_id", "artifacts", "decisions", "task_index", "task_items",
    )
    schema_: Literal["work-task-repair-prepare-request/v1"] = Field(alias="schema")
    stage: Literal["format", "complete"]
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    artifacts: TaskArtifactsModel
    decisions: list[TaskRepairDecisionModel] = Field(min_length=1)
    task_index: TaskIndexContract
    task_items: dict[str, TaskItemContract] = Field(min_length=1)

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
        if location in {("task_index",), ("task_items",)} and first["type"] != "missing":
            return WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_repair_candidate",
                             "TASK collection repair requires an explicit complete index and TASK item map.")
        return super()._work_error(error)


class TaskRepairRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-repair-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "stage", "requirement_id", "artifacts", "expected", "decisions", "task_index", "task_items",
    )
    schema_: Literal["work-task-repair-request/v1"] = Field(alias="schema")
    stage: Literal["format", "complete"]
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    artifacts: TaskArtifactsModel
    expected: dict[str, Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")] | None]
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
    approved_sha256: str = Field(pattern=r"^[0-9a-f]{64}$")
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
    "artifacts": _ARTIFACTS, "decisions": _DECISIONS,
    "task_index": TaskIndexContract.contract_example, "task_items": {"TASK-001": TaskItemContract.contract_example},
}
TaskRepairRequestContract.contract_example = {
    **TaskRepairPrepareRequestContract.contract_example, "schema": "work-task-repair-request/v1",
    "expected": {"outputs/work/tasks/example/index.json": "0" * 64},
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
