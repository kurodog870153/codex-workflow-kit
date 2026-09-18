from __future__ import annotations

from typing import Annotated, Any, ClassVar, Literal

from pydantic import BeforeValidator, Field

from ..models.common.base import WorkContract


def _tuple_input(value: object) -> object:
    return tuple(value) if isinstance(value, list) else value


StringTuple = Annotated[tuple[str, ...], BeforeValidator(_tuple_input)]


def order_hierarchy_selection(value: object) -> object:
    if not isinstance(value, dict):
        return value
    ordered = {
        field: value[field]
        for field in (
            "schema", "decision", "selected_paths", "entries",
            "catalog_sha256", "selection_sha256",
        )
        if field in value
    }
    for field in sorted(set(value) - set(ordered)):
        ordered[field] = value[field]
    if isinstance(ordered.get("entries"), list):
        entries: list[object] = []
        for raw_entry in ordered["entries"]:
            if not isinstance(raw_entry, dict):
                entries.append(raw_entry)
                continue
            entry = {
                field: raw_entry[field]
                for field in (
                    "path", "mode_support", "mode_metadata",
                    "recommendation_reason",
                )
                if field in raw_entry
            }
            for field in sorted(set(raw_entry) - set(entry)):
                entry[field] = raw_entry[field]
            if isinstance(entry.get("mode_metadata"), dict):
                metadata_by_mode: dict[str, object] = {}
                for mode in ("plan", "task", "execute"):
                    raw_metadata = entry["mode_metadata"].get(mode)
                    if not isinstance(raw_metadata, dict):
                        if raw_metadata is not None:
                            metadata_by_mode[mode] = raw_metadata
                        continue
                    metadata = {
                        field: raw_metadata[field]
                        for field in ("name", "description", "work_tags")
                        if field in raw_metadata
                    }
                    for field in sorted(set(raw_metadata) - set(metadata)):
                        metadata[field] = raw_metadata[field]
                    metadata_by_mode[mode] = metadata
                for mode in sorted(set(entry["mode_metadata"]) - set(metadata_by_mode)):
                    metadata_by_mode[mode] = entry["mode_metadata"][mode]
                entry["mode_metadata"] = metadata_by_mode
            entries.append(entry)
        ordered["entries"] = entries
    return ordered


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
