from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field

from ..common.base import WorkContract


ID_PREFIXES = {
    "goals": "GOAL",
    "scope": "SCOPE",
    "constraints": "CONSTRAINT",
    "dependencies": "DEPENDENCY",
    "risks": "RISK",
    "milestones": "MILESTONE",
    "deliverables": "DELIVERABLE",
    "acceptance_criteria": "ACCEPTANCE",
    "decisions": "PLAN-DECISION",
    "changes": "PLAN-CHANGE",
}
TOP_REQUIRED = {
    "schema",
    "requirement_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "hierarchy_selection",
    "work_instruction_selection",
    "skill_selection",
    "goals",
    "scope",
    "deliverables",
    "acceptance_criteria",
}
TOP_OPTIONAL = {
    "constraints",
    "dependencies",
    "risks",
    "milestones",
    "decisions",
    "changes",
}


class PlanPrepareNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class PlanGoalModel(PlanPrepareNestedModel):
    id: str
    statement: str


class PlanScopeModel(PlanPrepareNestedModel):
    id: str
    kind: Literal["in_scope", "out_of_scope"]
    statement: str
    goal_ids: list[str] | None = None


class PlanApplicableModel(PlanPrepareNestedModel):
    id: str
    statement: str
    applies_to: list[str]


class PlanRiskModel(PlanPrepareNestedModel):
    id: str
    condition: str
    impact: str
    mitigation: str
    applies_to: list[str]


class PlanMilestoneModel(PlanPrepareNestedModel):
    id: str
    statement: str
    deliverable_ids: list[str]


class PlanDeliverableModel(PlanPrepareNestedModel):
    id: str
    statement: str
    goal_ids: list[str]
    acceptance_ids: list[str]


class PlanAcceptanceModel(PlanPrepareNestedModel):
    id: str
    statement: str
    deliverable_ids: list[str]


class PlanDecisionModel(PlanPrepareNestedModel):
    id: str
    statement: str
    rationale: str
    applies_to: list[str]


class PlanPrepareContentModel(PlanPrepareNestedModel):
    title: str
    summary: str
    goals: list[PlanGoalModel]
    scope: list[PlanScopeModel]
    constraints: list[PlanApplicableModel] | None = None
    dependencies: list[PlanApplicableModel] | None = None
    risks: list[PlanRiskModel] | None = None
    milestones: list[PlanMilestoneModel] | None = None
    deliverables: list[PlanDeliverableModel]
    acceptance_criteria: list[PlanAcceptanceModel]
    decisions: list[PlanDecisionModel] | None = None


class PlanArtifactsModel(PlanPrepareNestedModel):
    plan: str
    task: str
    execution: str


class PlanPrepareRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-plan-prepare-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "requirement_id", "content", "hierarchy_selection", "skill_selection",
        "references", "artifacts",
    )
    field_references: ClassVar[dict[str, str]] = {
        "hierarchy_selection": "work-hierarchy-selection/v1",
        "skill_selection": "work-skill-selection/v1",
    }

    requirement_id: str
    content: PlanPrepareContentModel
    hierarchy_selection: dict[str, Any]
    skill_selection: dict[str, Any]
    references: list[str]
    artifacts: PlanArtifactsModel | None = None


PlanPrepareRequestContract.contract_example = {
    "requirement_id": "example",
    "content": {
        "title": "Example", "summary": "Example plan.",
        "goals": [{"id": "GOAL-001", "statement": "Deliver the result."}],
        "scope": [{"id": "SCOPE-001", "kind": "in_scope", "statement": "Implement the result.", "goal_ids": ["GOAL-001"]}],
        "constraints": [{"id": "CONSTRAINT-001", "statement": "Preserve compatibility.", "applies_to": ["PLAN"]}],
        "dependencies": [{"id": "DEPENDENCY-001", "statement": "Required source is available.", "applies_to": ["GOAL-001"]}],
        "risks": [{"id": "RISK-001", "condition": "Source changes.", "impact": "Validation fails.", "mitigation": "Revalidate sources.", "applies_to": ["PLAN"]}],
        "milestones": [{"id": "MILESTONE-001", "statement": "Result is ready.", "deliverable_ids": ["DELIVERABLE-001"]}],
        "deliverables": [{"id": "DELIVERABLE-001", "statement": "Completed result.", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}],
        "acceptance_criteria": [{"id": "ACCEPTANCE-001", "statement": "The result is verified.", "deliverable_ids": ["DELIVERABLE-001"]}],
        "decisions": [{"id": "PLAN-DECISION-001", "statement": "Use the current contract.", "rationale": "Keep one source of truth.", "applies_to": ["PLAN"]}],
    },
    "hierarchy_selection": {"schema": "work-hierarchy-selection/v1", "decision": "general_only", "selected_paths": [], "entries": [], "catalog_sha256": "0" * 64, "selection_sha256": "0" * 64},
    "skill_selection": {"schema": "work-skill-selection/v1", "decision": "base_only", "skills": [], "selection_sha256": "0" * 64},
    "references": [],
}


class PlanContract(WorkContract):
    contract_id: ClassVar[str] = "work-plan/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "status", "title", "summary", "artifacts",
        "hierarchy_selection", "work_instruction_selection", "skill_selection",
        "goals", "scope", "constraints", "dependencies", "risks", "milestones",
        "deliverables", "acceptance_criteria", "decisions", "changes",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed",
        "title": "Example", "summary": "Example plan.", "artifacts": {},
        "hierarchy_selection": {}, "work_instruction_selection": {},
        "skill_selection": {}, "goals": [], "scope": [], "deliverables": [],
        "acceptance_criteria": [],
    }
    schema_: Literal["work-plan/v1"] = Field(alias="schema")
    requirement_id: str
    status: Literal["confirmed"]
    title: str
    summary: str
    artifacts: dict[str, Any]
    hierarchy_selection: dict[str, Any]
    work_instruction_selection: dict[str, Any]
    skill_selection: dict[str, Any]
    goals: list[dict[str, Any]]
    scope: list[dict[str, Any]]
    constraints: list[dict[str, Any]] | None = None
    dependencies: list[dict[str, Any]] | None = None
    risks: list[dict[str, Any]] | None = None
    milestones: list[dict[str, Any]] | None = None
    deliverables: list[dict[str, Any]]
    acceptance_criteria: list[dict[str, Any]]
    decisions: list[dict[str, Any]] | None = None
    changes: list[dict[str, Any]] | None = None


class PlanValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-plan-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "status", "plan_sha256",
        "hierarchy_selection_sha256", "work_instructions_sha256",
        "skill_selection_sha256", "item_count",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-plan-validation/v1", "requirement_id": "example",
        "status": "confirmed", "plan_sha256": "0" * 64,
        "hierarchy_selection_sha256": "0" * 64,
        "work_instructions_sha256": "0" * 64,
        "skill_selection_sha256": "0" * 64, "item_count": 0,
    }
    schema_: Literal["work-plan-validation/v1"] = Field(alias="schema")
    requirement_id: str
    status: Literal["confirmed"]
    plan_sha256: str
    hierarchy_selection_sha256: str
    work_instructions_sha256: str
    skill_selection_sha256: str
    item_count: int


class PlanPrepareContract(WorkContract):
    contract_id: ClassVar[str] = "work-plan-prepare/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "status", "path", "plan", "validation")
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-plan-prepare/v1", "status": "prepared", "path": "outputs/work/plans/example.json",
        "plan": PlanContract.contract_example, "validation": PlanValidationContract.contract_example,
    }
    schema_: Literal["work-plan-prepare/v1"] = Field(alias="schema")
    status: Literal["prepared"]
    path: str
    plan: PlanContract
    validation: PlanValidationContract


class PlanCreateContract(PlanValidationContract):
    contract_id: ClassVar[str] = "work-plan-create/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "status", "plan_sha256",
        "hierarchy_selection_sha256", "work_instructions_sha256",
        "skill_selection_sha256", "item_count", "path",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        **PlanValidationContract.contract_example,
        "schema": "work-plan-create/v1", "path": "outputs/work/plans/example.json",
    }
    schema_: Literal["work-plan-create/v1"] = Field(alias="schema")
    path: str
