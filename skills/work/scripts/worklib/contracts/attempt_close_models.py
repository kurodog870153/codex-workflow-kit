from __future__ import annotations

from typing import ClassVar, Literal, Self

from pydantic import Field, ValidationError, model_validator

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from .base import WorkContract


class AttemptCloseRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-close-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "final_type", "reason", "authorization_evidence",
    )
    schema_: Literal["work-attempt-close-request/v1"] = Field(alias="schema")
    status: Literal["completed", "stopped", "blocked"]
    final_type: str | None = None
    reason: str | None = None
    authorization_evidence: str | None = None

    @model_validator(mode="after")
    def validate_final_details(self) -> Self:
        present = [name for name in ("final_type", "reason") if getattr(self, name) is not None]
        if self.status == "completed" and present:
            raise WorkError(
                ExitCode.CONTRACT,
                "attempt_close_unexpected_final_details",
                "A completed Attempt cannot include final_type or reason.",
                {"fields": present},
            )
        if self.status == "completed" and self.authorization_evidence is not None:
            raise WorkError(
                ExitCode.CONTRACT,
                "attempt_close_unexpected_authorization_evidence",
                "A completed Attempt reuses its manifest authorization.",
            )
        if self.status != "completed":
            missing = [name for name in ("final_type", "reason") if getattr(self, name) is None]
            if missing:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "attempt_close_missing_final_details",
                    "A stopped or blocked Attempt requires final_type and reason.",
                    {"missing": missing},
                )
            if not self.authorization_evidence or not self.authorization_evidence.strip():
                raise WorkError(
                    ExitCode.CONTRACT,
                    "attempt_close_missing_authorization_evidence",
                    "A stopped or blocked Attempt requires fresh authorization evidence.",
                )
            if not self.final_type.strip() or not self.reason.strip():  # type: ignore[union-attr]
                raise WorkError(
                    ExitCode.CONTRACT,
                    "attempt_close_empty_final_detail",
                    "final_type and reason must be non-empty strings.",
                )
        return self

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(ExitCode.CONTRACT, "attempt_close_expected_object", "A JSON object is required.")
        try:
            return cls.model_validate(value)
        except WorkError:
            raise
        except ValidationError as error:
            issues = error.errors(include_url=False, include_context=False, include_input=False)
            first = issues[0]
            location = tuple(first["loc"])
            if location == ("schema",) and first["type"] not in {"missing", "extra_forbidden"}:
                raise WorkError(ExitCode.CONTRACT, "attempt_close_invalid_schema", "The attempt-close request schema is invalid.") from error
            if location == ("status",) and first["type"] not in {"missing", "extra_forbidden"}:
                status = value.get("status")
                raise WorkError(
                    ExitCode.CONTRACT, "attempt_close_invalid_status",
                    "status must be completed, stopped, or blocked.", {"status": status},
                ) from error
            field_issues = [item for item in issues if item["type"] in {"missing", "extra_forbidden"}]
            if field_issues:
                raise WorkError(
                    ExitCode.CONTRACT, "attempt_close_invalid_fields",
                    "The attempt-close request has missing or unknown fields.",
                    {
                        "missing": sorted(str(item["loc"][-1]) for item in field_issues if item["type"] == "missing"),
                        "unknown": sorted(str(item["loc"][-1]) for item in field_issues if item["type"] == "extra_forbidden"),
                    },
                ) from error
            raise WorkError(ExitCode.CONTRACT, "attempt_close_invalid_fields", "The attempt-close request is invalid.") from error


class AttemptCloseContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-close/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "attempt_path", "index_path",
        "attempt_status", "task_status", "overall_status", "lock_status",
    )
    schema_: Literal["work-attempt-close/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str
    attempt_path: str
    index_path: str
    attempt_status: Literal["completed", "stopped", "blocked"]
    task_status: str
    overall_status: str
    lock_status: Literal["released"]


AttemptCloseRequestContract.contract_example = {
    "schema": "work-attempt-close-request/v1", "status": "completed",
}
AttemptCloseContract.contract_example = {
    "schema": "work-attempt-close/v1", "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "attempt_status": "completed", "task_status": "completed",
    "overall_status": "completed", "lock_status": "released",
}
