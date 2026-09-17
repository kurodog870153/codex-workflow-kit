from __future__ import annotations

from typing import Annotated, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field

from .base import WorkContract


class _InvocationEntry(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)


class WorkflowEntry(_InvocationEntry):
    kind: Literal["workflow"]


class ProgressResumeEntry(_InvocationEntry):
    kind: Literal["progress_resume"]
    requirement_id: str


class TaskPlanningEntry(_InvocationEntry):
    kind: Literal["task_planning"]
    requirement_id: str


InvocationEntry = Annotated[
    WorkflowEntry | ProgressResumeEntry | TaskPlanningEntry,
    Field(discriminator="kind"),
]


class InvocationContract(WorkContract):
    contract_id: ClassVar[str] = "work-invocation/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "mode", "request", "entry",
    )
    contract_example: ClassVar[dict[str, object]] = {
        "schema": "work-invocation/v1",
        "mode": "plan",
        "request": "Describe the requested change.",
        "entry": {"kind": "workflow"},
    }

    schema_: Literal["work-invocation/v1"] = Field(alias="schema")
    mode: Literal["plan", "task", "execute"]
    request: str
    entry: InvocationEntry
