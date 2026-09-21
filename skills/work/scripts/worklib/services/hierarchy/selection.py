from __future__ import annotations

from typing import Any

from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError


REQUEST_FIELDS = frozenset({"decision", "selections"})
REQUEST_SELECTION_FIELDS = frozenset({"path", "recommendation_reason"})


def _strict_object(value: object, *, location: str, fields: frozenset[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    missing = sorted(fields - set(value))
    unknown = sorted(set(value) - fields)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    return value


def _text(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(ExitCode.CONTRACT, "empty_text_value", "A non-empty string is required.", {"location": location})
    return value


def parse_hierarchy_selection_request(value: object) -> tuple[str, list[str], list[str]]:
    request = _strict_object(value, location="hierarchy_selection_request", fields=REQUEST_FIELDS)
    raw_selections = request["selections"]
    if not isinstance(raw_selections, list):
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selections", "Hierarchy selections must be an array.")
    decision = request["decision"]
    if decision not in {"instruction_paths", "general_only"}:
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selection_decision", "The hierarchy selection decision is invalid.", {"decision": decision})
    if (decision == "instruction_paths") != bool(raw_selections):
        raise WorkError(ExitCode.CONTRACT, "hierarchy_selection_decision_mismatch", "The hierarchy selection decision does not match its selected paths.")
    selected_paths: list[str] = []
    reasons: list[str] = []
    for index, raw_selection in enumerate(raw_selections):
        location = f"hierarchy_selection_request.selections[{index}]"
        selection = _strict_object(raw_selection, location=location, fields=REQUEST_SELECTION_FIELDS)
        selected_paths.append(_text(selection["path"], location=f"{location}.path"))
        reasons.append(_text(selection["recommendation_reason"], location=f"{location}.recommendation_reason"))
    return decision, selected_paths, reasons


def parse_hierarchy_selection_json(raw: bytes) -> object:
    return parse_json_contract(raw, source="stdin")


def build_hierarchy_selection_snapshot(
    decision: str,
    selected_paths: list[str],
    recommendation_reasons: list[str],
    catalog: dict[str, object],
) -> dict[str, object]:
    paths = catalog["paths"]
    children = catalog["children"]
    metadata = catalog["metadata"]
    catalog_sha256 = catalog["catalog_sha256"]
    assert isinstance(paths, list) and isinstance(children, dict)
    assert isinstance(metadata, dict) and isinstance(catalog_sha256, str)
    entries: list[dict[str, object]] = []
    for index, path in enumerate(selected_paths):
        if path not in paths:
            parent, separator, _ = path.rpartition("/")
            parent_path = parent if separator else "general"
            raise WorkError(ExitCode.CONTRACT, "hierarchy_selection_path_missing", "A selected hierarchy path does not exist in the cross-mode catalog.", {"path": path, "parent": parent_path, "valid_choices": children.get(parent_path, [])})
        path_metadata = metadata[path]
        assert isinstance(path_metadata, dict)
        entries.append({
            "path": path,
            "mode_support": path_metadata["mode_support"],
            "mode_metadata": path_metadata["modes"],
            "recommendation_reason": recommendation_reasons[index],
        })
    return {
        "schema": "work-hierarchy-selection/v1",
        "decision": decision,
        "selected_paths": selected_paths,
        "entries": entries,
        "catalog_sha256": catalog_sha256,
    }
