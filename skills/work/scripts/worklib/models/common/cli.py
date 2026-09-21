from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from .base import WorkContract


class CliResultContract(WorkContract):
    contract_id: ClassVar[str] = "work-cli-result/v1"
    contract_kind: ClassVar[Literal["envelope"]] = "envelope"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "reason_code", "message", "data",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-cli-result/v1",
        "status": "success",
        "reason_code": "ok",
        "message": "The command completed successfully.",
        "data": {},
    }

    schema_: Literal["work-cli-result/v1"] = Field(alias="schema")
    status: Literal["success", "already_completed", "rejected", "failed"]
    reason_code: str
    message: str
    data: Any


class ErrorContract(WorkContract):
    contract_id: ClassVar[str] = "work-error/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "code", "message", "details",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-error/v1",
        "code": "example_error",
        "message": "An example error occurred.",
        "details": {},
    }

    schema_: Literal["work-error/v1"] = Field(alias="schema")
    code: str
    message: str
    details: dict[str, Any]


__all__ = ["CliResultContract", "ErrorContract"]

