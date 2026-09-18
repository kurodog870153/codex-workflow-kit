from __future__ import annotations

import re
from typing import Annotated, Any, ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from ..models.common.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..models.common.base import WorkContract
from .command_correction import canonicalize_command_correction


class CommandCorrectionRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-command-correction-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "record_id", "original_command", "actual_command", "reason",
    )

    schema_: Literal["work-command-correction-request/v1"] = Field(alias="schema")
    record_id: str
    original_command: dict[str, Any]
    actual_command: dict[str, Any]
    reason: str

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "command_correction_expected_object",
                "A JSON object is required.",
            )
        try:
            request = cls.model_validate(value)
        except ValidationError as error:
            issues = error.errors(
                include_url=False, include_context=False, include_input=False
            )
            first = issues[0]
            location = tuple(first["loc"])
            if location == ("schema",) and first["type"] not in {
                "missing", "extra_forbidden",
            }:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "command_correction_invalid_schema",
                    "The command-correction request schema is invalid.",
                ) from error
            field_issues = [
                issue
                for issue in issues
                if issue["type"] in {"missing", "extra_forbidden"}
            ]
            if field_issues:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "command_correction_invalid_fields",
                    "The command-correction request has missing or unknown fields.",
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
                "command_correction_invalid_fields",
                "The command-correction request is invalid.",
            ) from error
        if not request.record_id.startswith("CMD-"):
            raise WorkError(
                ExitCode.CONTRACT,
                "command_correction_invalid_record_id",
                "record_id must identify a reserved CMD record.",
                {"record_id": request.record_id},
            )
        return request

    def to_execution_dict(self) -> dict[str, Any]:
        return {
            "schema": self.contract_id,
            "record_id": self.record_id,
            "correction": {
                "original_command": self.original_command,
                "actual_command": self.actual_command,
                "reason": self.reason,
            },
        }


class CommandCorrectionContract(WorkContract):
    contract_id: ClassVar[str] = "work-command-correction/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "attempt_id", "record_id", "index_path",
        "correction_status", "lock_status",
    )

    schema_: Literal["work-command-correction/v1"] = Field(alias="schema")
    task_id: str
    attempt_id: str
    record_id: str
    index_path: str
    correction_status: Literal["recorded"]
    lock_status: Literal["record_reserved"]


class CommandRunRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-command-run-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "attempt_id", "record_id", "timeout_seconds",
    )

    schema_: Literal["work-command-run-request/v1"] = Field(alias="schema")
    attempt_id: str
    record_id: str
    timeout_seconds: int

    @classmethod
    def parse_request(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "expected_object",
                "A JSON object is required.",
                {"location": "command_run"},
            )
        try:
            request = cls.model_validate(value)
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
                        "location": "command_run",
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
                    ExitCode.WORKFLOW_STATE,
                    "command_run_schema",
                    "Use work-command-run-request/v1.",
                    {},
                ) from error
            if location == ("timeout_seconds",):
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "command_run_timeout",
                    "timeout_seconds must be an integer from 1 to 3600.",
                    {},
                ) from error
            raise WorkError(
                ExitCode.WORKFLOW_STATE,
                "command_run_identity",
                "Supply explicit canonical Attempt and reserved CMD IDs.",
                {},
            ) from error
        if not re.fullmatch(r"ATTEMPT-[0-9]{3}", request.attempt_id) or not re.fullmatch(
            r"CMD-[0-9]{3}(?:#[1-9][0-9]*)?", request.record_id
        ):
            raise WorkError(
                ExitCode.WORKFLOW_STATE,
                "command_run_identity",
                "Supply explicit canonical Attempt and reserved CMD IDs.",
                {},
            )
        if not 1 <= request.timeout_seconds <= 3600:
            raise WorkError(
                ExitCode.WORKFLOW_STATE,
                "command_run_timeout",
                "timeout_seconds must be an integer from 1 to 3600.",
                {},
            )
        return request


class CommandInvocationModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)


class DirectCommandInvocationModel(CommandInvocationModel):
    kind: Literal["direct"]
    executable: str
    executable_sha256: Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]
    argv: list[str]


class WindowsBatchInvocationModel(CommandInvocationModel):
    kind: Literal["windows_batch"]
    launcher: str
    launcher_sha256: Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]
    script: str
    script_sha256: Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]
    arguments: list[str]
    command_line: str
    launcher_arguments: list[str]


CommandInvocation = Annotated[
    DirectCommandInvocationModel | WindowsBatchInvocationModel,
    Field(discriminator="kind"),
]


class CommandPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-command-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "request", "task_id", "working_directory", "execution",
        "invocation", "receipt_prefix", "sources", "approved_sha256",
    )

    schema_: Literal["work-command-preview/v1"] = Field(alias="schema")
    request: CommandRunRequestContract
    task_id: str
    working_directory: str
    execution: dict[str, Any]
    invocation: CommandInvocation
    receipt_prefix: str
    sources: dict[str, str]
    approved_sha256: str | None = None


class CommandStartedContract(WorkContract):
    contract_id: ClassVar[str] = "work-command-started/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "preview", "authorization_evidence",
    )

    schema_: Literal["work-command-started/v1"] = Field(alias="schema")
    preview: CommandPreviewContract
    authorization_evidence: str


class CommandResultContract(WorkContract):
    contract_id: ClassVar[str] = "work-command-result/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "approved_sha256", "record_id", "status", "exit_code",
        "stdout_tail", "stdout_truncated", "stderr_tail", "stderr_truncated",
        "receipt_prefix", "record_finish_required", "record_finish_request",
    )

    schema_: Literal["work-command-result/v1"] = Field(alias="schema")
    approved_sha256: str
    record_id: str
    status: Literal["exited", "timed_out", "launch_failed"]
    exit_code: int | None
    stdout_tail: str
    stdout_truncated: bool
    stderr_tail: str
    stderr_truncated: bool
    receipt_prefix: str | None = None
    record_finish_required: bool | None = None
    record_finish_request: dict[str, Any] | None = None


CommandCorrectionRequestContract.contract_example = {
    "schema": "work-command-correction-request/v1",
    "record_id": "CMD-001",
    "original_command": {"mode": "argv", "argv": ["tool", "old"]},
    "actual_command": {"mode": "argv", "argv": ["tool", "new"]},
    "reason": "Use the authorized argument.",
}
CommandCorrectionContract.contract_example = {
    "schema": "work-command-correction/v1", "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001", "record_id": "CMD-001",
    "index_path": "outputs/work/executions/example/index.json",
    "correction_status": "recorded", "lock_status": "record_reserved",
}
CommandRunRequestContract.contract_example = {
    "schema": "work-command-run-request/v1", "attempt_id": "ATTEMPT-001",
    "record_id": "CMD-001", "timeout_seconds": 60,
}
CommandPreviewContract.contract_example = {
    "schema": "work-command-preview/v1",
    "request": CommandRunRequestContract.contract_example,
    "task_id": "TASK-001",
    "working_directory": "/project", "execution": {"os": "linux"},
    "invocation": {
        "kind": "direct", "executable": "/usr/bin/tool",
        "executable_sha256": "0" * 64, "argv": ["tool", "--version"],
    },
    "receipt_prefix": "outputs/work/executions/example/TASK-001/ATTEMPT-001/.work-command-CMD-001",
    "sources": {"outputs/work/tasks/example/index.json": "0" * 64},
    "approved_sha256": "1" * 64,
}
CommandStartedContract.contract_example = {
    "schema": "work-command-started/v1",
    "preview": CommandPreviewContract.contract_example,
    "authorization_evidence": "User approved this command.",
}
CommandResultContract.contract_example = {
    "schema": "work-command-result/v1", "approved_sha256": "1" * 64,
    "record_id": "CMD-001", "status": "exited", "exit_code": 0,
    "stdout_tail": "", "stdout_truncated": False,
    "stderr_tail": "", "stderr_truncated": False,
}
