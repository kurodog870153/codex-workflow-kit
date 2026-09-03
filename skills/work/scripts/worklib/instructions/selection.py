from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import INSTRUCTION_SOURCE_KINDS
from .sources import InstructionSourceSet, load_instruction_sources


SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
SELECTION_FIELDS = {
    "selected_paths",
    "resolved_paths",
    "sources",
    "references",
    "instructions_sha256",
}
SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256"}


def build_instruction_selection(
    *,
    skill_root: Path,
    mode: str,
    selected_paths: list[str],
    reference_names: list[str] | None = None,
) -> dict[str, object]:
    loaded = load_instruction_sources(
        skill_root,
        mode,
        selected_paths,
        reference_names,
    )
    return {
        "selected_paths": list(loaded.hierarchy.selected_paths),
        "resolved_paths": list(loaded.hierarchy.resolved_paths),
        "sources": [source.as_dict() for source in loaded.sources],
        "references": list(loaded.references),
        "instructions_sha256": loaded.instructions_sha256,
    }


def _strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "expected_object",
            "A JSON object is required.",
            {"location": location},
        )
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - required)
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": location, "missing": missing, "unknown": unknown},
        )
    return value


def _string_array(
    value: object,
    *,
    location: str,
    allow_empty: bool,
) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A string array with the required cardinality is required.",
            {"location": location},
        )
    result: list[str] = []
    for index, item in enumerate(value):
        if not isinstance(item, str) or not item:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_string_array",
                "Every array item must be a non-empty string.",
                {"location": f"{location}[{index}]"},
            )
        result.append(item)
    return result


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            {"location": location},
        )
    return value


def validate_instruction_selection(
    value: object,
    *,
    skill_root: Path,
    mode: str,
    location: str = "instruction_selection",
) -> InstructionSourceSet:
    selection = _strict_object(
        value,
        location=location,
        required=SELECTION_FIELDS,
    )
    selected_paths = _string_array(
        selection["selected_paths"],
        location=f"{location}.selected_paths",
        allow_empty=True,
    )
    resolved_paths = _string_array(
        selection["resolved_paths"],
        location=f"{location}.resolved_paths",
        allow_empty=False,
    )
    references = _string_array(
        selection["references"],
        location=f"{location}.references",
        allow_empty=True,
    )
    if len(references) != len(set(references)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_instruction_reference",
            "Instruction reference logical names must be unique.",
            {"location": f"{location}.references"},
        )

    raw_sources = selection["sources"]
    if not isinstance(raw_sources, list) or not raw_sources:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_instruction_sources",
            "Instruction sources must be a non-empty array.",
            {"location": f"{location}.sources"},
        )
    sources: list[dict[str, str]] = []
    for index, raw_source in enumerate(raw_sources):
        source_location = f"{location}.sources[{index}]"
        source = _strict_object(
            raw_source,
            location=source_location,
            required=SOURCE_FIELDS,
        )
        kind = source["kind"]
        if kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_kind",
                "The instruction source kind is invalid.",
                {"location": f"{source_location}.kind", "kind": kind},
            )
        logical_name = source["logical_name"]
        if not isinstance(logical_name, str) or not logical_name:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_logical_name",
                "The instruction source logical name must be a non-empty string.",
                {"location": f"{source_location}.logical_name"},
            )
        canonical_sha256 = _sha256(
            source["canonical_sha256"],
            location=f"{source_location}.canonical_sha256",
        )
        sources.append(
            {
                "kind": kind,
                "logical_name": logical_name,
                "canonical_sha256": canonical_sha256,
            }
        )

    stored_fingerprint = _sha256(
        selection["instructions_sha256"],
        location=f"{location}.instructions_sha256",
    )
    current = load_instruction_sources(
        skill_root,
        mode,
        selected_paths,
        references,
    )
    if (
        list(current.hierarchy.selected_paths) != selected_paths
        or list(current.hierarchy.resolved_paths) != resolved_paths
    ):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_selection_hierarchy_mismatch",
            "The stored instruction hierarchy does not match the current resolution.",
            {"location": location},
        )
    if list(current.references) != references:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_selection_references_mismatch",
            "The stored instruction references are not in actual load order.",
            {"location": f"{location}.references"},
        )
    current_sources = [source.as_dict() for source in current.sources]
    if current_sources != sources:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_selection_sources_mismatch",
            "The stored instruction sources do not match the current sources.",
            {"location": f"{location}.sources"},
        )
    if current.instructions_sha256 != stored_fingerprint:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instructions_fingerprint_mismatch",
            "The stored instruction fingerprint does not match the current sources.",
            {"location": f"{location}.instructions_sha256"},
        )
    return current
