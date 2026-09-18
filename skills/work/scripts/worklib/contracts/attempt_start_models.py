from __future__ import annotations

from typing import ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, ValidationError, field_validator, model_validator

from ..models.common.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..models.common.base import WorkContract
from .attempt_authorization_models import AttemptAuthorizationContract


SHA256_PATTERN = r"^[0-9a-f]{64}$"
ATTEMPT_PATTERN = r"^ATTEMPT-\d{3}$"
RECORD_PATTERN = r"^(?:STEP|CMD|OP|VAL)-\d{3}(?:#\d+)?$"


class AttemptStartNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class CarriedRecordModel(AttemptStartNestedModel):
    record_id: str = Field(pattern=RECORD_PATTERN)
    evidence: str

    @field_validator("evidence")
    @classmethod
    def nonempty_evidence(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("Evidence must be non-empty.")
        return value


class AttemptContinuationModel(AttemptStartNestedModel):
    source_attempt_id: str = Field(pattern=ATTEMPT_PATTERN)
    carried_records: list[CarriedRecordModel]

    @model_validator(mode="after")
    def unique_records(self) -> Self:
        seen: set[str] = set()
        for record in self.carried_records:
            if record.record_id in seen:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "attempt_start_duplicate_carried_record",
                    "A carried record ID cannot be repeated.",
                    {"record_id": record.record_id},
                )
            seen.add(record.record_id)
        return self


class AttemptStartRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-start-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "worktree_snapshot_sha256", "authorization", "continuation",
    )
    schema_: Literal["work-attempt-start-request/v1"] = Field(alias="schema")
    worktree_snapshot_sha256: str = Field(pattern=SHA256_PATTERN)
    authorization: AttemptAuthorizationContract
    continuation: AttemptContinuationModel | None = None

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        try:
            return cls.model_validate(value)
        except WorkError:
            raise
        except ValidationError as error:
            issues = error.errors(include_url=False, include_context=False, include_input=False)
            first = issues[0]
            location = tuple(first["loc"])
            if location == ("schema",):
                code, message = "attempt_start_invalid_schema", "The Attempt-start request schema is invalid."
            elif location == ("worktree_snapshot_sha256",) and first["type"] not in {"missing", "extra_forbidden"}:
                code, message = "attempt_start_invalid_worktree_snapshot", "A lowercase worktree snapshot SHA-256 is required."
            elif location[-1:] == ("source_attempt_id",):
                code, message = "attempt_start_invalid_source_attempt", "The continuation source Attempt ID is invalid."
            elif location[-1:] == ("record_id",):
                code, message = "attempt_start_invalid_carried_record_id", "A carried record ID is invalid."
            elif location[-1:] == ("evidence",) and first["type"] not in {"missing", "extra_forbidden"}:
                code, message = "attempt_start_empty_text_value", "A non-empty string is required."
            elif location[-1:] == ("carried_records",) and first["type"] not in {"missing", "extra_forbidden"}:
                code, message = "attempt_start_invalid_carried_records", "carried_records must be an array."
            else:
                field_issues = [item for item in issues if item["type"] in {"missing", "extra_forbidden"}]
                if field_issues:
                    parent = tuple(field_issues[0]["loc"][:-1])
                    related = [item for item in field_issues if tuple(item["loc"][:-1]) == parent]
                    raise WorkError(
                        ExitCode.CONTRACT,
                        "attempt_start_invalid_object_fields",
                        "The JSON object has missing or unknown fields.",
                        {
                            "location": "attempt_start_request" if not parent else ".".join(map(str, parent)),
                            "missing": sorted(str(item["loc"][-1]) for item in related if item["type"] == "missing"),
                            "unknown": sorted(str(item["loc"][-1]) for item in related if item["type"] == "extra_forbidden"),
                        },
                    ) from error
                code, message = "attempt_start_invalid_object_fields", "The Attempt-start request is invalid."
            raise WorkError(ExitCode.CONTRACT, code, message) from error


class AttemptStartContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-start/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "attempt_path", "index_path", "status", "lock_status",
    )
    schema_: Literal["work-attempt-start/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str = Field(pattern=ATTEMPT_PATTERN)
    attempt_path: str
    index_path: str
    status: Literal["started"]
    lock_status: Literal["held"]


class AttemptStartRecoveryContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-start-recovery/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = AttemptStartContract.canonical_order
    schema_: Literal["work-attempt-start-recovery/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str = Field(pattern=ATTEMPT_PATTERN)
    attempt_path: str
    index_path: str
    status: Literal["recovered"]
    lock_status: Literal["held"]


AttemptStartRequestContract.contract_example = {
    "schema": "work-attempt-start-request/v1", "worktree_snapshot_sha256": "0" * 64,
    "authorization": AttemptAuthorizationContract.contract_example,
}
AttemptStartContract.contract_example = {
    "schema": "work-attempt-start/v1", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json", "status": "started", "lock_status": "held",
}
AttemptStartRecoveryContract.contract_example = {
    **AttemptStartContract.contract_example,
    "schema": "work-attempt-start-recovery/v1", "status": "recovered",
}
