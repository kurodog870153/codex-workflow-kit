from __future__ import annotations

from ..instructions.catalog import MODES


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
