from __future__ import annotations

from typing import Annotated, Any, ClassVar, Literal

from pydantic import BeforeValidator, Field

from ..common.base import WorkContract


StringTuple = Annotated[
    tuple[str, ...],
    BeforeValidator(lambda value: tuple(value) if isinstance(value, list) else value),
]


class HierarchyContract(WorkContract):
    contract_id: ClassVar[str] = "work-hierarchy/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "work_directory", "selected_paths", "resolved_paths",
        "required_paths", "optional_paths",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-hierarchy/v1", "work_directory": "plan",
        "selected_paths": [], "resolved_paths": ["general"],
        "required_paths": ["general"], "optional_paths": [],
    }

    schema_: Literal["work-hierarchy/v1"] = Field(alias="schema")
    work_directory: Literal["plan", "task", "execute"]
    selected_paths: StringTuple
    resolved_paths: StringTuple
    required_paths: StringTuple
    optional_paths: StringTuple

    def as_dict(self) -> dict[str, Any]:
        return self.to_canonical_dict()


class HierarchySelectionContract(WorkContract):
    contract_id: ClassVar[str] = "work-hierarchy-selection/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "decision", "selected_paths", "entries",
        "catalog_sha256", "selection_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-hierarchy-selection/v1", "decision": "general_only",
        "selected_paths": [], "entries": [], "catalog_sha256": "0" * 64,
        "selection_sha256": "0" * 64,
    }

    schema_: Literal["work-hierarchy-selection/v1"] = Field(alias="schema")
    decision: Literal["instruction_paths", "general_only"]
    selected_paths: list[str]
    entries: list[dict[str, Any]]
    catalog_sha256: str
    selection_sha256: str


class HierarchySelectionValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-hierarchy-selection-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "hierarchy_selection",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-hierarchy-selection-validation/v1", "status": "valid",
        "hierarchy_selection": HierarchySelectionContract.contract_example,
    }

    schema_: Literal["work-hierarchy-selection-validation/v1"] = Field(alias="schema")
    status: Literal["valid"]
    hierarchy_selection: HierarchySelectionContract
