from __future__ import annotations

import re
from pathlib import Path
from typing import Any, ClassVar, Literal, Self

from pydantic import Field, ValidationError, model_validator

from ..models.common.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..models.common.base import WorkContract


RecoveryTransaction = Literal[
    "record_begin", "command_correction", "record_finish", "attempt_close",
    "correction", "deviation_record",
]


class ExecutionRecoveryRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-recovery-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "transaction", "attempt_id", "transaction_files",
    )

    schema_: Literal["work-execution-recovery-request/v1"] = Field(alias="schema")
    transaction: RecoveryTransaction
    attempt_id: str
    transaction_files: list[str]

    @model_validator(mode="after")
    def validate_file_list(self) -> Self:
        if any(
            not item
            or Path(item).name != item
            or "/" in item
            or "\\" in item
            for item in self.transaction_files
        ):
            raise WorkError(
                ExitCode.CONTRACT,
                "execution_recovery_invalid_file_name",
                "Every transaction file must be a plain non-empty file name.",
            )
        if self.transaction_files != sorted(self.transaction_files) or len(
            self.transaction_files
        ) != len(set(self.transaction_files)):
            raise WorkError(
                ExitCode.CONTRACT,
                "execution_recovery_noncanonical_file_list",
                "transaction_files must be unique and sorted.",
            )
        return self

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "execution_recovery_expected_object",
                "A JSON object is required.",
            )
        try:
            request = cls.model_validate(value)
        except WorkError:
            raise
        except ValidationError as error:
            issues = error.errors(
                include_url=False, include_context=False, include_input=False
            )
            first = issues[0]
            location = tuple(first["loc"])
            field_issues = [
                issue
                for issue in issues
                if issue["type"] in {"missing", "extra_forbidden"}
            ]
            if field_issues:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "execution_recovery_invalid_fields",
                    "The execution-recovery request has missing or unknown fields.",
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
            codes = {
                "schema": (
                    "execution_recovery_invalid_schema",
                    "The execution-recovery request schema is invalid.",
                ),
                "transaction": (
                    "execution_recovery_invalid_transaction",
                    "transaction is not supported by general execution recovery.",
                ),
                "attempt_id": (
                    "execution_recovery_invalid_attempt_id",
                    "attempt_id must use the canonical ATTEMPT-nnn format.",
                ),
                "transaction_files": (
                    "execution_recovery_invalid_file_list",
                    "transaction_files must be an array.",
                ),
            }
            field = str(location[0])
            code, message = codes[field]
            details = (
                {"transaction": value.get("transaction")}
                if field == "transaction"
                else None
            )
            raise WorkError(ExitCode.CONTRACT, code, message, details) from error
        if not re.fullmatch(r"ATTEMPT-\d{3}", request.attempt_id):
            raise WorkError(
                ExitCode.CONTRACT,
                "execution_recovery_invalid_attempt_id",
                "attempt_id must use the canonical ATTEMPT-nnn format.",
            )
        return request


class ExecutionRecoveryPrepareRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-recovery-prepare-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "transaction", "attempt_id",
    )

    schema_: Literal["work-execution-recovery-prepare-request/v1"] = Field(alias="schema")
    transaction: RecoveryTransaction
    attempt_id: str

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT, "expected_object", "A JSON object is required.",
                {"location": "recovery_prepare"},
            )
        try:
            return cls.model_validate(value)
        except ValidationError as error:
            issues = error.errors(
                include_url=False, include_context=False, include_input=False
            )
            first = issues[0]
            location = tuple(first["loc"])
            field_issues = [
                issue
                for issue in issues
                if issue["type"] in {"missing", "extra_forbidden"}
            ]
            if field_issues:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_object_fields",
                    "The JSON object has missing or unknown fields.",
                    {
                        "location": "recovery_prepare",
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
            if location == ("schema",):
                raise WorkError(
                    ExitCode.ARTIFACT_INTEGRITY,
                    "recovery_prepare_schema",
                    "Use work-execution-recovery-prepare-request/v1.",
                ) from error
            recovery_value = {
                **value,
                "schema": "work-execution-recovery-request/v1",
                "transaction_files": [],
            }
            ExecutionRecoveryRequestContract.model_validate(recovery_value)
            raise

    def to_recovery_request(self) -> ExecutionRecoveryRequestContract:
        return ExecutionRecoveryRequestContract.model_validate(
            {
                "schema": "work-execution-recovery-request/v1",
                "transaction": self.transaction,
                "attempt_id": self.attempt_id,
                "transaction_files": [],
            }
        )


class RecoveryEvidenceContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-recovery-evidence/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = ("raw_sha256", "size_bytes")

    raw_sha256: str
    size_bytes: int


class ExecutionRecoveryPrepareContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-recovery-prepare/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "request", "task_id", "attempt_path", "index_path",
        "lock", "attempt_status", "evidence", "recovery_validation",
        "recovery_authorized",
    )

    schema_: Literal["work-execution-recovery-prepare/v1"] = Field(alias="schema")
    status: Literal["prepared"]
    request: ExecutionRecoveryRequestContract
    task_id: str
    attempt_path: str
    index_path: str
    lock: dict[str, Any] | None
    attempt_status: str
    evidence: dict[str, RecoveryEvidenceContract]
    recovery_validation: Literal["requires_authorized_recover"]
    recovery_authorized: Literal[False]


class ExecutionRecoveryContract(WorkContract):
    contract_id: ClassVar[str] = "work-execution-recovery/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "transaction", "task_id", "attempt_id", "attempt_path",
        "correction_id", "correction_path", "index_path", "affected_task_ids",
        "record_id", "attempt_status", "lock_status", "status",
    )

    schema_: Literal["work-execution-recovery/v1"] = Field(alias="schema")
    transaction: RecoveryTransaction
    task_id: str
    attempt_id: str
    attempt_path: str | None = None
    correction_id: str | None = None
    correction_path: str | None = None
    index_path: str
    affected_task_ids: list[str] | None = None
    record_id: str | None = None
    attempt_status: str | None = None
    lock_status: Literal["record_reserved", "attempt_held", "released"]
    status: Literal["recovered"]


ExecutionRecoveryRequestContract.contract_example = {
    "schema": "work-execution-recovery-request/v1",
    "transaction": "record_begin", "attempt_id": "ATTEMPT-001",
    "transaction_files": [".work-record-begin-TASK-001-ATTEMPT-001-VAL-001.tmp"],
}
ExecutionRecoveryPrepareRequestContract.contract_example = {
    "schema": "work-execution-recovery-prepare-request/v1",
    "transaction": "record_begin", "attempt_id": "ATTEMPT-001",
}
RecoveryEvidenceContract.contract_example = {
    "raw_sha256": "0" * 64, "size_bytes": 1,
}
ExecutionRecoveryPrepareContract.contract_example = {
    "schema": "work-execution-recovery-prepare/v1", "status": "prepared",
    "request": ExecutionRecoveryRequestContract.contract_example,
    "task_id": "TASK-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "lock": None, "attempt_status": "in_progress",
    "evidence": {"outputs/work/executions/example/index.json": RecoveryEvidenceContract.contract_example},
    "recovery_validation": "requires_authorized_recover",
    "recovery_authorized": False,
}
ExecutionRecoveryContract.contract_example = {
    "schema": "work-execution-recovery/v1", "transaction": "record_begin",
    "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "record_id": "VAL-001", "lock_status": "record_reserved",
    "status": "recovered",
}
