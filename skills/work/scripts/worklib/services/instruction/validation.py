from __future__ import annotations

import re
from typing import Any

from ...protocol import (
    INSTRUCTION_SOURCE_KINDS,
    INVALID_SHA256_ERROR_CODE,
    SHA256_PATTERN as SHA256_PATTERN_TEXT,
)
from ...models.common.errors import ExitCode, WorkError
from ...models.instruction import InstructionSourceSet

SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256"}
SELECTION_FIELDS = {"selected_paths", "resolved_paths", "sources", "references", "instructions_sha256"}
SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)


def strict_object(value: object, *, location: str, required: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    missing, unknown = sorted(required - set(value)), sorted(set(value) - required)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    return value


def string_array(value: object, *, location: str, allow_empty: bool) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise WorkError(ExitCode.CONTRACT, "invalid_string_array", "A string array with the required cardinality is required.", {"location": location})
    result: list[str] = []
    for index, item in enumerate(value):
        if not isinstance(item, str) or not item:
            raise WorkError(ExitCode.CONTRACT, "invalid_string_array", "Every array item must be a non-empty string.", {"location": f"{location}[{index}]"})
        result.append(item)
    return result


def sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": location})
    return value


def parse_instruction_selection(value: object, *, location: str = "instruction_selection") -> dict[str, object]:
    selection = strict_object(value, location=location, required=SELECTION_FIELDS)
    selected_paths = string_array(selection["selected_paths"], location=f"{location}.selected_paths", allow_empty=True)
    resolved_paths = string_array(selection["resolved_paths"], location=f"{location}.resolved_paths", allow_empty=False)
    references = string_array(selection["references"], location=f"{location}.references", allow_empty=True)
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Instruction reference logical names must be unique.", {"location": f"{location}.references"})
    raw_sources = selection["sources"]
    if not isinstance(raw_sources, list) or not raw_sources:
        raise WorkError(ExitCode.CONTRACT, "invalid_instruction_sources", "Instruction sources must be a non-empty array.", {"location": f"{location}.sources"})
    sources: list[dict[str, str]] = []
    for index, raw_source in enumerate(raw_sources):
        source_location = f"{location}.sources[{index}]"
        source = strict_object(raw_source, location=source_location, required=SOURCE_FIELDS)
        kind, logical_name = source["kind"], source["logical_name"]
        if kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_kind", "The instruction source kind is invalid.", {"location": f"{source_location}.kind", "kind": kind})
        if not isinstance(logical_name, str) or not logical_name:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_logical_name", "The instruction source logical name must be a non-empty string.", {"location": f"{source_location}.logical_name"})
        sources.append({"kind": kind, "logical_name": logical_name, "canonical_sha256": sha256(source["canonical_sha256"], location=f"{source_location}.canonical_sha256")})
    return {"selected_paths": selected_paths, "resolved_paths": resolved_paths, "sources": sources, "references": references, "instructions_sha256": sha256(selection["instructions_sha256"], location=f"{location}.instructions_sha256")}


def validate_instruction_selection(value: object, current: InstructionSourceSet, *, location: str = "instruction_selection") -> InstructionSourceSet:
    selection = parse_instruction_selection(value, location=location)
    if list(current.hierarchy.selected_paths) != selection["selected_paths"] or list(current.hierarchy.resolved_paths) != selection["resolved_paths"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_selection_hierarchy_mismatch", "The stored instruction hierarchy does not match the current resolution.", {"location": location})
    if list(current.references) != selection["references"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_selection_references_mismatch", "The stored instruction references are not in actual load order.", {"location": f"{location}.references"})
    if [source.as_dict() for source in current.sources] != selection["sources"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_selection_sources_mismatch", "The stored instruction sources do not match the current sources.", {"location": f"{location}.sources"})
    if current.instructions_sha256 != selection["instructions_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instructions_fingerprint_mismatch", "The stored instruction fingerprint does not match the current sources.", {"location": f"{location}.instructions_sha256"})
    return current


__all__ = ["parse_instruction_selection", "sha256", "strict_object", "string_array", "validate_instruction_selection"]
