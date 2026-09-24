from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from ..common.base import WorkContract


DelegationRole = Literal[
    "plan", "task-coordinator", "execute", "task-skill",
    "artifact-editor", "progress-saver",
]
DelegationMode = Literal["plan", "task", "execute"]


class DelegationEnvelopeContract(WorkContract):
    contract_id: ClassVar[str] = "work-delegation-envelope/v1"
    contract_kind: ClassVar[Literal["generated_request"]] = "generated_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "marker", "skill", "role", "sender", "mode",
        "project_root", "skill_root", "request", "context",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-delegation-envelope/v1",
        "marker": "WORK_DELEGATION_V1", "skill": "$work", "role": "plan",
        "sender": "parent", "mode": "plan", "project_root": "C:/project",
        "skill_root": "C:/work", "request": "Confirmed role request.",
        "context": {},
    }

    schema_: Literal["work-delegation-envelope/v1"] = Field(alias="schema")
    marker: str
    skill: Literal["$work"]
    role: DelegationRole
    sender: Literal["parent", "task-coordinator"]
    mode: DelegationMode
    project_root: str
    skill_root: str
    request: str
    context: dict[str, Any]


class DelegationBuildRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-delegation-build-request/v1"
    contract_kind: ClassVar[Literal["semantic_request"]] = "semantic_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "role", "mode", "request", "source_plan_path", "task_id",
        "source_progress_path", "content", "confirmed_request", "decisions",
        "affected_task_ids", "repository_evidence", "saved_discussion",
        "continuation_point", "save_approval",
    )

    schema_: Literal["work-delegation-build-request/v1"] = Field(alias="schema")
    role: DelegationRole
    mode: DelegationMode | None = None
    request: str = Field(min_length=1, pattern=r"\S")
    source_plan_path: str | None = None
    task_id: str | None = None
    source_progress_path: str | None = None
    content: dict[str, Any] | None = None
    confirmed_request: dict[str, Any] | None = None
    decisions: list[Any] | None = None
    affected_task_ids: list[str] | None = None
    repository_evidence: list[str] | None = None
    saved_discussion: list[str] | None = None
    continuation_point: str | None = None
    save_approval: str | None = None


DelegationBuildRequestContract.contract_example = {
    "schema": "work-delegation-build-request/v1", "role": "task-coordinator",
    "request": "Coordinate the confirmed TASK work.",
    "source_plan_path": "outputs/work/plans/example.json",
}


class DelegationValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-delegation-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "role", "mode", "scope", "source_validation",
        "sender_authentication", "grants_authorization",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-delegation-validation/v1", "status": "valid",
        "role": "plan", "mode": "plan", "scope": "role_context",
        "source_validation": "not_checked",
        "sender_authentication": "not_checked", "grants_authorization": False,
    }

    schema_: Literal["work-delegation-validation/v1"] = Field(alias="schema")
    status: Literal["valid"]
    role: DelegationRole
    mode: DelegationMode
    scope: Literal["discussion_restoration", "role_context"]
    source_validation: Literal["not_checked"]
    sender_authentication: Literal["not_checked"]
    grants_authorization: Literal[False]
