from __future__ import annotations

import re
from typing import ClassVar, Literal

from pydantic import Field, ValidationError

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from .base import WorkContract


class CorrectionCreateRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-correction-create-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "target_attempt_id", "field", "correct_value", "reason",
        "invalidates_completion",
    )

    schema_: Literal["work-correction-create-request/v1"] = Field(alias="schema")
    target_attempt_id: str
    field: str
    correct_value: str
    reason: str
    invalidates_completion: bool

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> "CorrectionCreateRequestContract":
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "correction_create_expected_object",
                "A JSON object is required.",
            )
        try:
            request = cls.model_validate(value)
        except ValidationError as error:
            issues = error.errors(include_url=False, include_context=False, include_input=False)
            first = issues[0]
            location = tuple(first["loc"])
            if location == ("schema",) and first["type"] not in {"missing", "extra_forbidden"}:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "correction_create_invalid_schema",
                    "The Correction create request schema is invalid.",
                ) from error
            if location == ("invalidates_completion",) and first["type"] not in {"missing", "extra_forbidden"}:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "correction_create_invalid_invalidation_flag",
                    "invalidates_completion must be a boolean.",
                ) from error
            field_issues = [
                issue for issue in issues
                if issue["type"] in {"missing", "extra_forbidden"}
            ]
            if field_issues:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "correction_create_invalid_fields",
                    "The Correction create request has missing or unknown fields.",
                    {
                        "missing": sorted(
                            str(issue["loc"][-1]) for issue in field_issues
                            if issue["type"] == "missing"
                        ),
                        "unknown": sorted(
                            str(issue["loc"][-1]) for issue in field_issues
                            if issue["type"] == "extra_forbidden"
                        ),
                    },
                ) from error
            field = str(first["loc"][-1])
            if field in {"field", "correct_value", "reason"}:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "correction_create_empty_text",
                    "Correction text fields must be non-empty strings.",
                    {"field": field},
                ) from error
            raise
        if not re.fullmatch(r"ATTEMPT-\d{3}", request.target_attempt_id):
            raise WorkError(
                ExitCode.CONTRACT,
                "correction_create_invalid_attempt_id",
                "target_attempt_id must use ATTEMPT-nnn.",
            )
        for field in ("field", "correct_value", "reason"):
            if not getattr(request, field).strip():
                raise WorkError(
                    ExitCode.CONTRACT,
                    "correction_create_empty_text",
                    "Correction text fields must be non-empty strings.",
                    {"field": field},
                )
        return request


class CorrectionCreateContract(WorkContract):
    contract_id: ClassVar[str] = "work-correction-create/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "correction_id", "correction_path",
        "index_path", "affected_task_ids", "lock_status",
    )

    schema_: Literal["work-correction-create/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str
    correction_id: str
    correction_path: str
    index_path: str
    affected_task_ids: list[str]
    lock_status: Literal["released"]


CorrectionCreateRequestContract.contract_example = {
    "schema": "work-correction-create-request/v1",
    "target_attempt_id": "ATTEMPT-001",
    "field": "records[0].outcome",
    "correct_value": "passed",
    "reason": "Correct the recorded outcome.",
    "invalidates_completion": True,
}
CorrectionCreateContract.contract_example = {
    "schema": "work-correction-create/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "correction_id": "ATTEMPT-001-CORRECTION-001",
    "correction_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/corrections/ATTEMPT-001-CORRECTION-001.json",
    "index_path": "outputs/work/executions/example/index.json",
    "affected_task_ids": ["TASK-001"],
    "lock_status": "released",
}
