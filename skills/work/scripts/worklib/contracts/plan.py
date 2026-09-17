from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from .base import WorkContract


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
