from __future__ import annotations

from ...technical.foundation.fingerprint import canonical_json_sha256


def hierarchy_selection_sha256(
    decision: str,
    selected_paths: list[str],
    entries: list[dict[str, object]],
    catalog_sha256: str,
) -> str:
    return canonical_json_sha256({
        "decision": decision,
        "selected_paths": selected_paths,
        "entries": entries,
        "catalog_sha256": catalog_sha256,
    })
