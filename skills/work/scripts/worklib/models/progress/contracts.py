from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from ..common.base import WorkContract


FIELDS = (
    "schema", "requirement_id", "mode", "revision", "status", "title", "request",
    "current_task_id", "context", "source_status", "notes", "confirmed_decisions",
    "tentative", "open_questions", "next_discussion_point",
)


class DiscussionProgressContract(WorkContract):
    contract_id: ClassVar[str] = "work-discussion-progress/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = FIELDS
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-discussion-progress/v1", "requirement_id": "example",
        "mode": "plan", "revision": 1, "status": "discussion_only",
        "title": "Example", "request": "Example request.", "current_task_id": None,
        "context": {}, "source_status": [], "notes": [], "confirmed_decisions": [],
        "tentative": [], "open_questions": [], "next_discussion_point": "Continue.",
    }
    schema_: Literal["work-discussion-progress/v1"] = Field(alias="schema")
    requirement_id: str
    mode: Literal["plan", "task"]
    revision: int
    status: Literal["discussion_only"]
    title: str
    request: str
    current_task_id: str | None
    context: dict[str, Any]
    source_status: list[str]
    notes: list[str]
    confirmed_decisions: list[dict[str, str]]
    tentative: list[str]
    open_questions: list[str]
    next_discussion_point: str


class ProgressReadContract(WorkContract):
    contract_id: ClassVar[str] = "work-progress-read/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "status", "path", "sha256", "progress")
    contract_example: ClassVar[dict[str, Any]] = {"schema": "work-progress-read/v1", "status": "saved", "path": "outputs/work/progress/example/plan/progress.json", "sha256": "0" * 64, "progress": DiscussionProgressContract.contract_example}
    schema_: Literal["work-progress-read/v1"] = Field(alias="schema")
    status: Literal["saved"]
    path: str
    sha256: str
    progress: DiscussionProgressContract


class ProgressPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-progress-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "status", "path", "expected_revision", "approved_sha256", "progress", "source_validation", "evidence_trust", "formal_readiness")
    contract_example: ClassVar[dict[str, Any]] = {"schema": "work-progress-preview/v1", "status": "valid", "path": "outputs/work/progress/example/plan/progress.json", "expected_revision": 0, "approved_sha256": "0" * 64, "progress": DiscussionProgressContract.contract_example, "source_validation": "not_checked", "evidence_trust": "historical_context_only", "formal_readiness": "not_established"}
    schema_: Literal["work-progress-preview/v1"] = Field(alias="schema")
    status: Literal["valid"]
    path: str
    expected_revision: int
    approved_sha256: str
    progress: DiscussionProgressContract
    source_validation: Literal["not_checked"]
    evidence_trust: Literal["historical_context_only"]
    formal_readiness: Literal["not_established"]


class ProgressPrepareContract(ProgressPreviewContract):
    contract_id: ClassVar[str] = "work-progress-prepare/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ProgressPreviewContract.canonical_order
    contract_example: ClassVar[dict[str, Any]] = {**ProgressPreviewContract.contract_example, "schema": "work-progress-prepare/v1"}
    schema_: Literal["work-progress-prepare/v1"] = Field(alias="schema")


class ProgressSaveRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-progress-save-request/v1"
    contract_kind: ClassVar[Literal["generated_request"]] = "generated_request"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "path", "expected_revision", "previous_sha256", "progress")
    contract_example: ClassVar[dict[str, Any]] = {"schema": "work-progress-save-request/v1", "path": "outputs/work/progress/example/plan/progress.json", "expected_revision": 0, "previous_sha256": None, "progress": DiscussionProgressContract.contract_example}
    schema_: Literal["work-progress-save-request/v1"] = Field(alias="schema")
    path: str
    expected_revision: int
    previous_sha256: str | None
    progress: DiscussionProgressContract


class ProgressSaveContract(ProgressReadContract):
    contract_id: ClassVar[str] = "work-progress-save/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "status", "path", "sha256", "progress", "approved_sha256")
    contract_example: ClassVar[dict[str, Any]] = {**ProgressReadContract.contract_example, "schema": "work-progress-save/v1", "approved_sha256": "0" * 64}
    schema_: Literal["work-progress-save/v1"] = Field(alias="schema")
    approved_sha256: str

