from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from ..common.base import WorkContract
from ..hierarchy import HierarchyContract


class InstructionCatalogContract(WorkContract):
    contract_id: ClassVar[str] = "work-instruction-catalog/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "mode", "paths", "children", "metadata", "catalog_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-instruction-catalog/v1", "mode": "plan",
        "paths": ["general"], "children": {"general": []},
        "metadata": {}, "catalog_sha256": "0" * 64,
    }

    schema_: Literal["work-instruction-catalog/v1"] = Field(alias="schema")
    mode: Literal["plan", "task", "execute", "all"]
    paths: list[str]
    children: dict[str, list[str]]
    metadata: dict[str, Any]
    catalog_sha256: str


class InstructionsContract(WorkContract):
    contract_id: ClassVar[str] = "work-instructions/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "mode", "hierarchy", "sources", "references",
        "instructions_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-instructions/v1", "mode": "plan",
        "hierarchy": HierarchyContract.contract_example,
        "sources": [], "references": [], "instructions_sha256": "0" * 64,
    }

    schema_: Literal["work-instructions/v1"] = Field(alias="schema")
    mode: Literal["plan", "task", "execute"]
    hierarchy: HierarchyContract
    sources: list[dict[str, str | int]]
    references: list[str]
    instructions_sha256: str


class InstructionSelectionContract(WorkContract):
    contract_id: ClassVar[str] = "work-instruction-selection/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "mode", "instruction_selection",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-instruction-selection/v1", "mode": "plan",
        "instruction_selection": {
            "selected_paths": [], "resolved_paths": ["general"], "sources": [],
            "references": [], "instructions_sha256": "0" * 64,
        },
    }

    schema_: Literal["work-instruction-selection/v1"] = Field(alias="schema")
    mode: Literal["plan", "task", "execute"]
    instruction_selection: dict[str, Any]
