from __future__ import annotations

import re
from typing import Any, ClassVar, Literal

from pydantic import Field

from ...protocol import (
    ATTEMPT_ID_PATTERN as ATTEMPT_ID_PATTERN_TEXT,
    SHA256_PATTERN as SHA256_PATTERN_TEXT,
    TASK_ID_PATTERN as TASK_ID_PATTERN_TEXT,
)
from ..common.base import WorkContract



HANDOFF_MARKER = "WORK-HANDOFF"
SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)
TASK_SPEC_PATTERN = re.compile(r"^TASK-SPEC-\d{3}$")
TASK_PATTERN = re.compile(TASK_ID_PATTERN_TEXT)
ATTEMPT_PATTERN = re.compile(ATTEMPT_ID_PATTERN_TEXT)
AFFECTED_ID_PATTERN = re.compile(r"^[A-Z][A-Z0-9-]*-\d{3}$")
DIRECTION_STAGES = {
    "plan_to_task": ("plan", "task"),
    "task_to_execute": ("task", "execute"),
    "execute_to_task": ("execute", "task"),
    "task_to_plan": ("task", "plan"),
    "execute_to_plan": ("execute", "plan"),
}
RETURN_DIRECTIONS = {"execute_to_task", "task_to_plan", "execute_to_plan"}
COMMON_FIELDS = {
    "schema",
    "marker",
    "direction",
    "requirement_id",
    "artifacts",
    "source",
    "target",
    "summary",
}
RETURN_FIELDS = {
    "confirmed_approach",
    "requested_changes",
    "preserve",
    "affected_ids",
    "validation_requirements",
}


class HandoffContract(WorkContract):
    contract_id: ClassVar[str] = "work-handoff/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "marker", "direction", "requirement_id", "artifacts", "source",
        "target", "summary", "confirmed_approach", "requested_changes", "preserve",
        "affected_ids", "validation_requirements",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-handoff/v1", "marker": "WORK-HANDOFF",
        "direction": "plan_to_task", "requirement_id": "example", "artifacts": {},
        "source": {}, "target": {}, "summary": "Continue.", "affected_ids": ["GOAL-001"],
    }
    schema_: Literal["work-handoff/v1"] = Field(alias="schema")
    marker: Literal["WORK-HANDOFF"]
    direction: Literal["plan_to_task", "task_to_execute", "execute_to_task", "task_to_plan", "execute_to_plan"]
    requirement_id: str
    artifacts: dict[str, str]
    source: dict[str, Any]
    target: dict[str, Any]
    summary: str
    confirmed_approach: str | None = None
    requested_changes: list[str] | None = None
    preserve: list[str] | None = None
    affected_ids: list[str] | None = None
    validation_requirements: list[str] | None = None


class DiscussionHandoffRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-discussion-handoff-request/v1"
    contract_kind: ClassVar[Literal["semantic_request"]] = "semantic_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "direction", "requirement_id", "summary", "confirmed_approach",
        "requested_changes", "preserve", "affected_ids", "validation_requirements",
    )
    schema_: Literal["work-discussion-handoff-request/v1"] = Field(alias="schema")
    direction: Literal["plan_to_task", "task_to_plan"]
    requirement_id: str = Field(min_length=1, pattern=r"\S")
    summary: str = Field(min_length=1, pattern=r"\S")
    confirmed_approach: str | None = None
    requested_changes: list[str] | None = None
    preserve: list[str] | None = None
    affected_ids: list[str] | None = None
    validation_requirements: list[str] | None = None


class DiscussionHandoffContract(WorkContract):
    contract_id: ClassVar[str] = "work-discussion-handoff/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "marker", "direction", "requirement_id", "source_stage",
        "target_stage", "source_status", "source_validation", "grants_authorization",
        "summary", "confirmed_approach", "requested_changes", "preserve",
        "affected_ids", "validation_requirements",
    )
    schema_: Literal["work-discussion-handoff/v1"] = Field(alias="schema")
    marker: Literal["WORK-DISCUSSION-HANDOFF"]
    direction: Literal["plan_to_task", "task_to_plan"]
    requirement_id: str
    source_stage: Literal["plan", "task"]
    target_stage: Literal["plan", "task"]
    source_status: Literal["unsaved_discussion"]
    source_validation: Literal["not_checked"]
    grants_authorization: Literal[False]
    summary: str
    confirmed_approach: str | None = None
    requested_changes: list[str] | None = None
    preserve: list[str] | None = None
    affected_ids: list[str] | None = None
    validation_requirements: list[str] | None = None


DiscussionHandoffRequestContract.contract_example = {
    "schema": "work-discussion-handoff-request/v1", "direction": "task_to_plan",
    "requirement_id": "example", "summary": "Review the unfinished discussion.",
}
DiscussionHandoffContract.contract_example = {
    "schema": "work-discussion-handoff/v1", "marker": "WORK-DISCUSSION-HANDOFF",
    "direction": "task_to_plan", "requirement_id": "example",
    "source_stage": "task", "target_stage": "plan",
    "source_status": "unsaved_discussion", "source_validation": "not_checked",
    "grants_authorization": False, "summary": "Review the unfinished discussion.",
}


class HandoffValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-handoff-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "marker", "direction", "requirement_id", "source_stage", "target_stage", "status")
    contract_example: ClassVar[dict[str, Any]] = {"schema": "work-handoff-validation/v1", "marker": "WORK-HANDOFF", "direction": "plan_to_task", "requirement_id": "example", "source_stage": "plan", "target_stage": "task", "status": "valid"}
    schema_: Literal["work-handoff-validation/v1"] = Field(alias="schema")
    marker: Literal["WORK-HANDOFF"]
    direction: str
    requirement_id: str
    source_stage: str
    target_stage: str
    status: Literal["valid"]


class HandoffSourceValidationContract(HandoffValidationContract):
    contract_id: ClassVar[str] = "work-handoff-source-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "marker", "direction", "requirement_id", "source_stage", "target_stage", "status", "plan_path", "task_path", "source")
    contract_example: ClassVar[dict[str, Any]] = {**HandoffValidationContract.contract_example, "schema": "work-handoff-source-validation/v1", "plan_path": "outputs/work/plans/example.json", "source": {}}
    schema_: Literal["work-handoff-source-validation/v1"] = Field(alias="schema")
    plan_path: str | None = None
    task_path: str | None = None
    source: dict[str, Any]
