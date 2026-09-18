from __future__ import annotations

from typing import Any, ClassVar, Literal, Self

from pydantic import Field, ValidationError, model_validator

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from .base import WorkContract


class RecordFinishRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-record-finish-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "record", "modified_files", "authorization_evidence",
    )

    schema_: Literal["work-record-finish-request/v1"] = Field(alias="schema")
    record: dict[str, Any]
    modified_files: list[str] | None = None
    authorization_evidence: str | None = None

    @model_validator(mode="after")
    def validate_modified_files(self) -> Self:
        if self.modified_files is not None:
            if not self.modified_files:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "record_finish_invalid_modified_files",
                    "modified_files must be a non-empty array when present.",
                )
            if any(not item for item in self.modified_files):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "record_finish_invalid_modified_file",
                    "Every modified file must be a non-empty string.",
                )
        return self

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "record_finish_expected_object",
                "A JSON object is required.",
            )
        try:
            return cls.model_validate(value)
        except WorkError:
            raise
        except ValidationError as error:
            issues = error.errors(
                include_url=False,
                include_context=False,
                include_input=False,
            )
            first = issues[0]
            location = tuple(first["loc"])
            if location == ("schema",) and first["type"] not in {
                "missing", "extra_forbidden",
            }:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "record_finish_invalid_schema",
                    "The record-finish request schema is invalid.",
                ) from error
            if location == ("record",) and first["type"] not in {
                "missing", "extra_forbidden",
            }:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "record_finish_invalid_record",
                    "record must be a JSON object.",
                ) from error
            if location and location[0] == "modified_files" and first["type"] not in {
                "missing", "extra_forbidden",
            }:
                code = (
                    "record_finish_invalid_modified_file"
                    if len(location) > 1
                    else "record_finish_invalid_modified_files"
                )
                message = (
                    "Every modified file must be a non-empty string."
                    if len(location) > 1
                    else "modified_files must be a non-empty array when present."
                )
                raise WorkError(ExitCode.CONTRACT, code, message) from error
            field_issues = [
                issue
                for issue in issues
                if issue["type"] in {"missing", "extra_forbidden"}
            ]
            if field_issues:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "record_finish_invalid_fields",
                    "The record-finish request has missing or unknown fields.",
                    {
                        "missing": sorted(
                            str(issue["loc"][-1])
                            for issue in field_issues
                            if issue["type"] == "missing"
                        ),
                        "unknown": sorted(
                            str(issue["loc"][-1])
                            for issue in field_issues
                            if issue["type"] == "extra_forbidden"
                        ),
                    },
                ) from error
            raise WorkError(
                ExitCode.CONTRACT,
                "record_finish_invalid_fields",
                "The record-finish request is invalid.",
            ) from error


class RecordBeginContract(WorkContract):
    contract_id: ClassVar[str] = "work-record-begin/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "base_record_id", "record_id",
        "record_kind", "index_path", "lock_status",
    )

    schema_: Literal["work-record-begin/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str
    base_record_id: str
    record_id: str
    record_kind: Literal["command", "operation", "validation"]
    index_path: str
    lock_status: Literal["record_reserved"]


class RecordFinishContract(WorkContract):
    contract_id: ClassVar[str] = "work-record-finish/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "record_id", "record_kind",
        "attempt_path", "index_path", "record_status", "lock_status",
    )

    schema_: Literal["work-record-finish/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str
    record_id: str
    record_kind: Literal["command", "operation", "validation"]
    attempt_path: str
    index_path: str
    record_status: Literal["recorded"]
    lock_status: Literal["attempt_held"]


RecordFinishRequestContract.contract_example = {
    "schema": "work-record-finish-request/v1",
    "record": {"id": "VAL-001", "kind": "validation", "outcome": "success"},
}
RecordBeginContract.contract_example = {
    "schema": "work-record-begin/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "base_record_id": "VAL-001",
    "record_id": "VAL-001",
    "record_kind": "validation",
    "index_path": "outputs/work/executions/example/index.json",
    "lock_status": "record_reserved",
}
RecordFinishContract.contract_example = {
    "schema": "work-record-finish/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "record_id": "VAL-001",
    "record_kind": "validation",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "record_status": "recorded",
    "lock_status": "attempt_held",
}
