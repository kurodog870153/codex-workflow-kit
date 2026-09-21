from __future__ import annotations

import re
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...protocol import INVALID_SHA256_ERROR_CODE, SHA256_PATTERN as SHA256_PATTERN_TEXT


SELECTION_FIELDS = frozenset({"schema", "decision", "selected_paths", "entries", "catalog_sha256", "selection_sha256"})
ENTRY_FIELDS = frozenset({"path", "mode_support", "mode_metadata", "recommendation_reason"})
SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)


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


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": location})
    return value


def validate_hierarchy_selection_snapshot(value: object, catalog: dict[str, object]) -> tuple[str, list[str], list[dict[str, object]], str]:
    selection = _strict_object(value, location="hierarchy_selection", fields=SELECTION_FIELDS)
    if selection["schema"] != "work-hierarchy-selection/v1":
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selection_schema", "The hierarchy selection schema is invalid.")
    raw_paths, raw_entries = selection["selected_paths"], selection["entries"]
    if not isinstance(raw_paths, list) or not all(isinstance(path, str) for path in raw_paths):
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selected_paths", "Hierarchy selected_paths must be an array of strings.")
    if not isinstance(raw_entries, list) or len(raw_entries) != len(raw_paths):
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selection_entries", "Hierarchy selection entries must align with selected_paths.")
    decision = selection["decision"]
    if decision not in {"instruction_paths", "general_only"}:
        raise WorkError(ExitCode.CONTRACT, "invalid_hierarchy_selection_decision", "The hierarchy selection decision is invalid.", {"decision": decision})
    if (decision == "instruction_paths") != bool(raw_paths):
        raise WorkError(ExitCode.CONTRACT, "hierarchy_selection_decision_mismatch", "The hierarchy selection decision does not match its selected paths.")
    current_catalog_sha256 = catalog["catalog_sha256"]
    assert isinstance(current_catalog_sha256, str)
    if _sha256(selection["catalog_sha256"], location="hierarchy_selection.catalog_sha256") != current_catalog_sha256:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_catalog_snapshot_mismatch", "The instruction catalog no longer matches its confirmed snapshot; return to Plan.")
    metadata = catalog["metadata"]
    assert isinstance(metadata, dict)
    entries: list[dict[str, object]] = []
    for index, raw_entry in enumerate(raw_entries):
        location = f"hierarchy_selection.entries[{index}]"
        entry = _strict_object(raw_entry, location=location, fields=ENTRY_FIELDS)
        path = _text(entry["path"], location=f"{location}.path")
        reason = _text(entry["recommendation_reason"], location=f"{location}.recommendation_reason")
        if path != raw_paths[index]:
            raise WorkError(ExitCode.CONTRACT, "hierarchy_selection_entry_order_mismatch", "Hierarchy selection entries must match selected_paths order.", {"location": location})
        path_metadata = metadata.get(path)
        expected = None if not isinstance(path_metadata, dict) else {
            "path": path, "mode_support": path_metadata["mode_support"],
            "mode_metadata": path_metadata["modes"], "recommendation_reason": reason,
        }
        if entry != expected:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "hierarchy_selection_metadata_mismatch", "A selected hierarchy path no longer matches its confirmed metadata; return to Plan.", {"path": path})
        entries.append(dict(entry))
    return decision, list(raw_paths), entries, _sha256(selection["selection_sha256"], location="hierarchy_selection.selection_sha256")


def validate_task_hierarchy_authorization(selected_paths: tuple[str, ...], confirmed_selection: object, *, location: str) -> tuple[str, ...]:
    if not isinstance(confirmed_selection, dict) or not isinstance(confirmed_selection.get("selected_paths"), list):
        raise WorkError(ExitCode.CONTRACT, "invalid_confirmed_hierarchy_selection", "The source Plan hierarchy selection is invalid.")
    allowed_paths: set[str] = set()
    for confirmed_path in confirmed_selection["selected_paths"]:
        if not isinstance(confirmed_path, str):
            raise WorkError(ExitCode.CONTRACT, "invalid_confirmed_hierarchy_selection", "The source Plan hierarchy selection is invalid.")
        parts = confirmed_path.split("/")
        allowed_paths.update("/".join(parts[:depth]) for depth in range(1, len(parts) + 1))
    unauthorized = [path for path in selected_paths if path not in allowed_paths and not any(path.startswith(f"{confirmed_path}/") for confirmed_path in confirmed_selection["selected_paths"])]
    if unauthorized:
        raise WorkError(ExitCode.CONTRACT, "task_hierarchy_path_not_authorized", "A TASK hierarchy path is not authorized by the source Plan.", {"location": location, "paths": unauthorized})
    return selected_paths
