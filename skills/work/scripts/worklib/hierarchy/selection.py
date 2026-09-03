from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.hierarchy import build_hierarchy
from ..foundation.markdown import parse_json_contract
from ..skills.fingerprint import canonical_json_sha256
from ..instructions.catalog import (
    MODES,
    build_cross_mode_instruction_catalog,
    resolve_instruction_hierarchy,
)


DECISIONS = frozenset({"instruction_paths", "general_only"})
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
REQUEST_FIELDS = frozenset({"decision", "selections"})
REQUEST_SELECTION_FIELDS = frozenset({"path", "recommendation_reason"})
SELECTION_FIELDS = frozenset(
    {
        "schema",
        "decision",
        "selected_paths",
        "entries",
        "catalog_sha256",
        "selection_sha256",
    }
)
ENTRY_FIELDS = frozenset(
    {
        "path",
        "mode_support",
        "mode_metadata",
        "recommendation_reason",
    }
)
METADATA_FIELDS = frozenset({"name", "description", "work_tags"})


def order_hierarchy_selection(value: object) -> object:
    if not isinstance(value, dict):
        return value
    ordered = {
        field: value[field]
        for field in (
            "schema",
            "decision",
            "selected_paths",
            "entries",
            "catalog_sha256",
            "selection_sha256",
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
                    "path",
                    "mode_support",
                    "mode_metadata",
                    "recommendation_reason",
                )
                if field in raw_entry
            }
            for field in sorted(set(raw_entry) - set(entry)):
                entry[field] = raw_entry[field]
            if isinstance(entry.get("mode_metadata"), dict):
                mode_metadata: dict[str, object] = {}
                for mode in MODES:
                    raw_metadata = entry["mode_metadata"].get(mode)
                    if not isinstance(raw_metadata, dict):
                        if raw_metadata is not None:
                            mode_metadata[mode] = raw_metadata
                        continue
                    metadata = {
                        field: raw_metadata[field]
                        for field in ("name", "description", "work_tags")
                        if field in raw_metadata
                    }
                    for field in sorted(set(raw_metadata) - set(metadata)):
                        metadata[field] = raw_metadata[field]
                    mode_metadata[mode] = metadata
                for mode in sorted(set(entry["mode_metadata"]) - set(mode_metadata)):
                    mode_metadata[mode] = entry["mode_metadata"][mode]
                entry["mode_metadata"] = mode_metadata
            entries.append(entry)
        ordered["entries"] = entries
    return ordered


def _strict_object(
    value: object,
    *,
    location: str,
    fields: frozenset[str],
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "expected_object",
            "A JSON object is required.",
            {"location": location},
        )
    missing = sorted(fields - set(value))
    unknown = sorted(set(value) - fields)
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": location, "missing": missing, "unknown": unknown},
        )
    return value


def _text(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(
            ExitCode.CONTRACT,
            "empty_text_value",
            "A non-empty string is required.",
            {"location": location},
        )
    return value


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            {"location": location},
        )
    return value


def hierarchy_selection_sha256(
    decision: str,
    selected_paths: list[str],
    entries: list[dict[str, object]],
    catalog_sha256: str,
) -> str:
    return canonical_json_sha256(
        {
            "decision": decision,
            "selected_paths": selected_paths,
            "entries": entries,
            "catalog_sha256": catalog_sha256,
        }
    )


def _validate_decision(decision: object, *, has_selections: bool) -> str:
    if decision not in DECISIONS:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_selection_decision",
            "The hierarchy selection decision is invalid.",
            {"decision": decision},
        )
    if (decision == "instruction_paths") != has_selections:
        raise WorkError(
            ExitCode.CONTRACT,
            "hierarchy_selection_decision_mismatch",
            "The hierarchy selection decision does not match its selected paths.",
        )
    return decision


def _catalog_snapshot(skill_root: Path) -> tuple[dict[str, object], str]:
    catalog = build_cross_mode_instruction_catalog(skill_root).as_dict()
    catalog_sha256 = catalog["catalog_sha256"]
    assert isinstance(catalog_sha256, str)
    return catalog, catalog_sha256


def _snapshot_entry(
    *,
    catalog: dict[str, object],
    path: str,
    recommendation_reason: str,
) -> dict[str, object]:
    paths = catalog["paths"]
    children = catalog["children"]
    metadata = catalog["metadata"]
    assert isinstance(paths, list)
    assert isinstance(children, dict)
    assert isinstance(metadata, dict)
    if path not in paths:
        parent, separator, _ = path.rpartition("/")
        parent_path = parent if separator else "general"
        raise WorkError(
            ExitCode.CONTRACT,
            "hierarchy_selection_path_missing",
            "A selected hierarchy path does not exist in the cross-mode catalog.",
            {
                "path": path,
                "parent": parent_path,
                "valid_choices": children.get(parent_path, []),
            },
        )
    if children[path]:
        raise WorkError(
            ExitCode.CONTRACT,
            "hierarchy_selection_path_not_leaf",
            "A selected hierarchy path must be a cross-mode catalog leaf.",
            {"path": path, "children": children[path]},
        )
    path_metadata = metadata[path]
    assert isinstance(path_metadata, dict)
    return {
        "path": path,
        "mode_support": path_metadata["mode_support"],
        "mode_metadata": path_metadata["modes"],
        "recommendation_reason": recommendation_reason,
    }


def build_hierarchy_selection(
    value: object,
    *,
    skill_root: Path,
) -> dict[str, object]:
    request = _strict_object(
        value,
        location="hierarchy_selection_request",
        fields=REQUEST_FIELDS,
    )
    raw_selections = request["selections"]
    if not isinstance(raw_selections, list):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_selections",
            "Hierarchy selections must be an array.",
        )
    decision = _validate_decision(
        request["decision"],
        has_selections=bool(raw_selections),
    )

    selected_paths: list[str] = []
    recommendation_reasons: list[str] = []
    for index, raw_selection in enumerate(raw_selections):
        location = f"hierarchy_selection_request.selections[{index}]"
        selection = _strict_object(
            raw_selection,
            location=location,
            fields=REQUEST_SELECTION_FIELDS,
        )
        selected_paths.append(_text(selection["path"], location=f"{location}.path"))
        recommendation_reasons.append(
            _text(
                selection["recommendation_reason"],
                location=f"{location}.recommendation_reason",
            )
        )
    build_hierarchy("plan", selected_paths)

    catalog, catalog_sha256 = _catalog_snapshot(skill_root)
    entries = [
        _snapshot_entry(
            catalog=catalog,
            path=path,
            recommendation_reason=recommendation_reasons[index],
        )
        for index, path in enumerate(selected_paths)
    ]
    selection_sha256 = hierarchy_selection_sha256(
        decision,
        selected_paths,
        entries,
        catalog_sha256,
    )
    return {
        "schema": "work-hierarchy-selection/v1",
        "decision": decision,
        "selected_paths": selected_paths,
        "entries": entries,
        "catalog_sha256": catalog_sha256,
        "selection_sha256": selection_sha256,
    }


def validate_hierarchy_selection(
    value: object,
    *,
    skill_root: Path,
) -> dict[str, object]:
    selection = _strict_object(
        value,
        location="hierarchy_selection",
        fields=SELECTION_FIELDS,
    )
    if selection["schema"] != "work-hierarchy-selection/v1":
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_selection_schema",
            "The hierarchy selection schema is invalid.",
        )
    raw_paths = selection["selected_paths"]
    raw_entries = selection["entries"]
    if not isinstance(raw_paths, list) or not all(
        isinstance(path, str) for path in raw_paths
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_selected_paths",
            "Hierarchy selected_paths must be an array of strings.",
        )
    if not isinstance(raw_entries, list) or len(raw_entries) != len(raw_paths):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_selection_entries",
            "Hierarchy selection entries must align with selected_paths.",
        )
    selected_paths = list(raw_paths)
    decision = _validate_decision(
        selection["decision"],
        has_selections=bool(selected_paths),
    )
    build_hierarchy("plan", selected_paths)

    catalog, current_catalog_sha256 = _catalog_snapshot(skill_root)
    stored_catalog_sha256 = _sha256(
        selection["catalog_sha256"],
        location="hierarchy_selection.catalog_sha256",
    )
    if stored_catalog_sha256 != current_catalog_sha256:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_catalog_snapshot_mismatch",
            "The instruction catalog no longer matches its confirmed snapshot; return to Plan.",
        )

    entries: list[dict[str, object]] = []
    for index, raw_entry in enumerate(raw_entries):
        location = f"hierarchy_selection.entries[{index}]"
        entry = _strict_object(raw_entry, location=location, fields=ENTRY_FIELDS)
        path = _text(entry["path"], location=f"{location}.path")
        reason = _text(
            entry["recommendation_reason"],
            location=f"{location}.recommendation_reason",
        )
        if path != selected_paths[index]:
            raise WorkError(
                ExitCode.CONTRACT,
                "hierarchy_selection_entry_order_mismatch",
                "Hierarchy selection entries must match selected_paths order.",
                {"location": location},
            )
        expected = _snapshot_entry(
            catalog=catalog,
            path=path,
            recommendation_reason=reason,
        )
        if entry != expected:
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "hierarchy_selection_metadata_mismatch",
                "A selected hierarchy path no longer matches its confirmed metadata; return to Plan.",
                {"path": path},
            )
        entries.append(dict(entry))

    stored_selection_sha256 = _sha256(
        selection["selection_sha256"],
        location="hierarchy_selection.selection_sha256",
    )
    current_selection_sha256 = hierarchy_selection_sha256(
        decision,
        selected_paths,
        entries,
        current_catalog_sha256,
    )
    if stored_selection_sha256 != current_selection_sha256:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "hierarchy_selection_fingerprint_mismatch",
            "The hierarchy selection fingerprint does not match its contents.",
        )
    return {
        "schema": "work-hierarchy-selection-validation/v1",
        "status": "valid",
        "hierarchy_selection": dict(selection),
    }


def build_hierarchy_selection_json(
    raw: bytes,
    *,
    skill_root: Path,
) -> dict[str, object]:
    return build_hierarchy_selection(
        parse_json_contract(raw, source="stdin"),
        skill_root=skill_root,
    )


def validate_hierarchy_selection_json(
    raw: bytes,
    *,
    skill_root: Path,
) -> dict[str, object]:
    return validate_hierarchy_selection(
        parse_json_contract(raw, source="stdin"),
        skill_root=skill_root,
    )


def validate_task_hierarchy_paths(
    selected_paths: object,
    *,
    confirmed_selection: object,
    skill_root: Path,
    location: str,
) -> tuple[str, ...]:
    if not isinstance(selected_paths, list) or any(
        not isinstance(path, str) for path in selected_paths
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_hierarchy_selected_paths",
            "Hierarchy selected_paths must be an array of strings.",
            {"location": location},
        )
    hierarchy = build_hierarchy("task", selected_paths)
    if not isinstance(confirmed_selection, dict) or not isinstance(
        confirmed_selection.get("selected_paths"), list
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_confirmed_hierarchy_selection",
            "The source Plan hierarchy selection is invalid.",
        )

    allowed_paths: set[str] = set()
    for confirmed_path in confirmed_selection["selected_paths"]:
        if not isinstance(confirmed_path, str):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_confirmed_hierarchy_selection",
                "The source Plan hierarchy selection is invalid.",
            )
        parts = confirmed_path.split("/")
        allowed_paths.update(
            "/".join(parts[:depth]) for depth in range(1, len(parts) + 1)
        )
    unauthorized = [
        path for path in hierarchy.selected_paths if path not in allowed_paths
    ]
    if unauthorized:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_hierarchy_path_not_authorized",
            "A TASK hierarchy path is not authorized by the source Plan.",
            {"location": location, "paths": unauthorized},
        )
    for mode in ("task", "execute"):
        resolve_instruction_hierarchy(skill_root, mode, list(hierarchy.selected_paths))
    return hierarchy.selected_paths
