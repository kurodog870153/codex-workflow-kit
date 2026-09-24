from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field

from ..common.base import WorkContract


class DraftNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class PlanningSourceModel(DraftNestedModel):
    plan_sha256: str
    hierarchy_selection_sha256: str
    skill_selection_sha256: str


class DraftInstructionSelectionModel(DraftNestedModel):
    selected_paths: list[str]
    references: list[str]


class DraftReferenceModel(DraftNestedModel):
    save_revision: int
    revision: int
    sha256: str


class PlanningTaskModel(DraftNestedModel):
    id: str
    title: str
    goal: str
    scope: list[str]
    skill_id: str | None
    dependencies: list[str]
    status: Literal["planned", "in_progress", "refined", "needs_review"]
    boundary_revision: int
    instructions_sha256: str
    draft_ref: DraftReferenceModel | None = None
    instruction_selection: DraftInstructionSelectionModel | None = None


class TaskPlanningIndexContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-planning-index/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "revision", "source", "current_task_id",
        "tasks", "retired_task_ids",
    )
    schema_: Literal["work-task-planning-index/v1"] = Field(alias="schema")
    requirement_id: str
    revision: int
    source: PlanningSourceModel
    current_task_id: str | None
    tasks: list[PlanningTaskModel]
    retired_task_ids: list[str] | None = None


class ConfirmedDecisionModel(DraftNestedModel):
    statement: str
    rationale: str


class TaskDraftContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-draft/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "task_id", "revision", "boundary_revision",
        "source", "instructions_sha256", "status", "notes",
        "confirmed_decisions", "tentative", "open_questions",
        "next_discussion_point", "task_candidate",
    )
    schema_: Literal["work-task-draft/v1"] = Field(alias="schema")
    requirement_id: str
    task_id: str
    revision: int
    boundary_revision: int
    source: PlanningSourceModel
    instructions_sha256: str
    status: Literal["in_progress", "refined", "needs_review"]
    notes: list[str]
    confirmed_decisions: list[ConfirmedDecisionModel]
    tentative: list[str]
    open_questions: list[str]
    next_discussion_point: str | None
    task_candidate: dict[str, Any] | None = None


class TaskPlanningIndexValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-planning-index-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "revision", "task_count", "task_order", "status",
    )
    schema_: Literal["work-task-planning-index-validation/v1"] = Field(alias="schema")
    requirement_id: str
    revision: int
    task_count: int
    task_order: list[str]
    status: Literal["valid"]


class TaskDraftValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-draft-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "task_id", "revision", "planning_status", "status",
    )
    schema_: Literal["work-task-draft-validation/v1"] = Field(alias="schema")
    requirement_id: str
    task_id: str
    revision: int
    planning_status: Literal["in_progress", "refined", "needs_review"]
    status: Literal["valid"]


class TaskDraftPrepareContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-draft-prepare/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "status", "request", "index", "affected_task_ids", "drafts")
    schema_: Literal["work-task-draft-prepare/v1"] = Field(alias="schema")
    status: Literal["prepared"]
    request: dict[str, Any]
    index: dict[str, Any]
    affected_task_ids: list[str]
    drafts: dict[str, dict[str, Any]]


class TaskSemanticReferenceModel(DraftNestedModel):
    existing_task_id: str | None = None
    upsert_position: int | None = None


class TaskSemanticItemModel(DraftNestedModel):
    existing_task_id: str | None = None
    title: str
    goal: str
    scope: list[str]
    skill_id: str | None
    instruction_selection: DraftInstructionSelectionModel | None = None
    dependencies: list[TaskSemanticReferenceModel]


class TaskSemanticRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-semantic-request/v1"
    contract_kind: ClassVar[Literal["semantic_request"]] = "semantic_request"
    canonical_order: ClassVar[tuple[str, ...]] = ("upsert", "remove_task_ids", "current_task", "reason")
    upsert: list[TaskSemanticItemModel]
    remove_task_ids: list[str]
    current_task: TaskSemanticReferenceModel | None
    reason: str | None


TaskSemanticRequestContract.contract_example = {
    "upsert": [{"title": "Implement", "goal": "Deliver the result.", "scope": ["Source"],
               "skill_id": None, "instruction_selection": {"selected_paths": [], "references": []},
               "dependencies": []}],
    "remove_task_ids": [], "current_task": {"upsert_position": 1}, "reason": None,
}


class TaskDraftSaveContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-draft-save/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "revision", "status", "mirror_status",
    )
    schema_: Literal["work-task-draft-save/v1"] = Field(alias="schema")
    requirement_id: str
    revision: int
    status: Literal["saved"]
    mirror_status: Literal["not_applicable", "updated", "superseded", "stale"]


class TaskDraftRecoveryContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-draft-recovery/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "requirement_id", "revision", "status", "display_copy")
    schema_: Literal["work-task-draft-recovery/v1"] = Field(alias="schema")
    requirement_id: str
    revision: int
    status: Literal["recovered", "already_completed"]
    display_copy: Literal["not_updated", "not_applicable"]


class TaskDraftSourceCheckContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-draft-source-check/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "status", "requirement_id", "task_id", "revision", "source", "skill_id", "instructions_sha256", "instruction_selection")
    schema_: Literal["work-task-draft-source-check/v1"] = Field(alias="schema")
    status: Literal["valid"]
    requirement_id: str
    task_id: str
    revision: int
    source: PlanningSourceModel
    skill_id: str | None
    instructions_sha256: str
    instruction_selection: DraftInstructionSelectionModel


_SOURCE_EXAMPLE = {
    "plan_sha256": "a" * 64,
    "hierarchy_selection_sha256": "b" * 64,
    "skill_selection_sha256": "c" * 64,
}
TaskPlanningIndexContract.contract_example = {
    "schema": "work-task-planning-index/v1", "requirement_id": "example",
    "revision": 1, "source": _SOURCE_EXAMPLE, "current_task_id": "TASK-001",
    "tasks": [{
        "id": "TASK-001", "title": "Example", "goal": "Deliver the result.",
        "scope": ["Implementation"], "skill_id": None, "dependencies": [],
        "status": "planned", "boundary_revision": 1, "instructions_sha256": "d" * 64,
    }],
}
TaskDraftContract.contract_example = {
    "schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001",
    "revision": 1, "boundary_revision": 1, "source": _SOURCE_EXAMPLE,
    "instructions_sha256": "d" * 64, "status": "in_progress", "notes": [],
    "confirmed_decisions": [], "tentative": [], "open_questions": [],
    "next_discussion_point": "Continue discussion.",
}
TaskPlanningIndexValidationContract.contract_example = {
    "schema": "work-task-planning-index-validation/v1", "requirement_id": "example",
    "revision": 1, "task_count": 1, "task_order": ["TASK-001"], "status": "valid",
}
TaskDraftValidationContract.contract_example = {
    "schema": "work-task-draft-validation/v1", "requirement_id": "example",
    "task_id": "TASK-001", "revision": 1, "planning_status": "in_progress", "status": "valid",
}
TaskDraftPrepareContract.contract_example = {"schema": "work-task-draft-prepare/v1", "status": "prepared", "request": {}, "index": {}, "affected_task_ids": [], "drafts": {}}
TaskDraftSaveContract.contract_example = {"schema": "work-task-draft-save/v1", "requirement_id": "example", "revision": 1, "status": "saved", "mirror_status": "not_applicable"}
TaskDraftRecoveryContract.contract_example = {"schema": "work-task-draft-recovery/v1", "requirement_id": "example", "revision": 1, "status": "recovered", "display_copy": "not_applicable"}
TaskDraftSourceCheckContract.contract_example = {"schema": "work-task-draft-source-check/v1", "status": "valid", "requirement_id": "example", "task_id": "TASK-001", "revision": 1, "source": _SOURCE_EXAMPLE, "skill_id": None, "instructions_sha256": "d" * 64, "instruction_selection": {"selected_paths": [], "references": []}}
