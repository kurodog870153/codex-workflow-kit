"""Discussion memory, independent of formal specification readiness."""

from __future__ import annotations

import re
from typing import Any, ClassVar, Literal

from pydantic import Field

from .base import WorkContract

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import validate_requirement_id
from .validation import nonempty_string, strict_keys


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
    contract_kind: ClassVar[Literal["request"]] = "request"
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


def validate_progress_contract(value: object) -> dict[str, Any]:
    progress = strict_keys(value, location="progress", required=set(FIELDS))
    if progress["schema"] != "work-discussion-progress/v1" or progress["status"] != "discussion_only":
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_schema", "Progress must be discussion-only memory.")
    validate_requirement_id(nonempty_string(progress["requirement_id"], location="requirement_id"))
    if progress["mode"] not in ("plan", "task"):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_mode", "Only Plan and Task discussions can be saved.")
    if type(progress["revision"]) is not int or progress["revision"] < 1:
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_revision", "A positive integer revision is required.")
    task_id = progress["current_task_id"]
    if task_id is not None and (
        progress["mode"] != "task" or not isinstance(task_id, str)
        or not re.fullmatch(r"TASK-[0-9]{3}", task_id)
    ):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_task", "Only Task progress may identify a current TASK-NNN.")
    for field in ("title", "request", "next_discussion_point"):
        nonempty_string(progress[field], location=field)
    if not isinstance(progress["context"], dict):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_context", "Supplied context must be a JSON object.")
    for field in ("source_status", "notes", "tentative", "open_questions", "confirmed_decisions"):
        if not isinstance(progress[field], list):
            raise WorkError(ExitCode.CONTRACT, "invalid_progress_list", "Discussion entries must be arrays.", {"field": field})
        for number, item in enumerate(progress[field]):
            location = f"{field}[{number}]"
            if field == "confirmed_decisions":
                decision = strict_keys(item, location=location, required={"statement"}, optional={"rationale"})
                nonempty_string(decision["statement"], location=location + ".statement")
                if "rationale" in decision:
                    nonempty_string(decision["rationale"], location=location + ".rationale")
            else:
                nonempty_string(item, location=location)
    # Context is inert historical data. Do not validate live Plan, TASK, skills or
    # instruction snapshots here: unavailable or stale sources are why saving is needed.
    ordered = {field: progress[field] for field in FIELDS}
    return parse_json_contract(render_json_contract(ordered), source="progress candidate")
