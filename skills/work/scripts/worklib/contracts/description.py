from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field

from ..models.common.base import ContractKind, WorkContract


class ContractCatalogEntry(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)

    id: str
    kind: ContractKind


class ContractFieldDescription(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)

    name: str
    required: bool
    type: str
    constraints: dict[str, Any]
    reference: str | None = None


class ContractCatalog(WorkContract):
    contract_id: ClassVar[str] = "work-contract-catalog/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "contracts")
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-contract-catalog/v1",
        "contracts": [],
    }

    schema_: Literal["work-contract-catalog/v1"] = Field(alias="schema")
    contracts: list[ContractCatalogEntry]


class ContractDescription(WorkContract):
    contract_id: ClassVar[str] = "work-contract-description/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema",
        "id",
        "kind",
        "required",
        "optional",
        "canonical_order",
        "fields",
        "example",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-contract-description/v1",
        "id": "work-contract-catalog/v1",
        "kind": "response",
        "required": ["schema", "contracts"],
        "optional": [],
        "canonical_order": ["schema", "contracts"],
        "fields": [],
        "example": {"schema": "work-contract-catalog/v1", "contracts": []},
    }

    schema_: Literal["work-contract-description/v1"] = Field(alias="schema")
    id: str
    kind: ContractKind
    required: list[str]
    optional: list[str]
    canonical_order_: list[str] = Field(alias="canonical_order")
    fields: list[ContractFieldDescription]
    example: dict[str, Any]


class ContractScaffold(WorkContract):
    contract_id: ClassVar[str] = "work-contract-scaffold/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema",
        "id",
        "canonical_order",
        "scaffold",
        "example",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-contract-scaffold/v1",
        "id": "work-attempt-close-request/v1",
        "canonical_order": [
            "schema", "status", "final_type", "reason", "authorization_evidence",
        ],
        "scaffold": {
            "schema": "work-attempt-close-request/v1",
            "status": "completed",
            "final_type": None,
            "reason": None,
            "authorization_evidence": None,
        },
        "example": {
            "schema": "work-attempt-close-request/v1",
            "status": "completed",
        },
    }

    schema_: Literal["work-contract-scaffold/v1"] = Field(alias="schema")
    id: str
    canonical_order_: list[str] = Field(alias="canonical_order")
    scaffold: dict[str, Any]
    example: dict[str, Any]
