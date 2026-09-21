from __future__ import annotations

from ...models.common.errors import ExitCode, WorkError
from ...models.instruction import InstructionSourceSet
from ...protocol import INSTRUCTION_SOURCE_KINDS, INVALID_SHA256_ERROR_CODE, SHA256_PATTERN

FIELDS = {"selected_paths", "resolved_paths", "sources", "references", "instructions_sha256"}
SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256"}


def build_work_instruction_selection(loaded: InstructionSourceSet) -> dict[str, object]:
    return {"selected_paths": list(loaded.hierarchy.selected_paths), "resolved_paths": list(loaded.hierarchy.resolved_paths), "sources": [source.as_dict() for source in loaded.sources], "references": list(loaded.references), "instructions_sha256": loaded.instructions_sha256}


def parse_work_instruction_selection(value: object, *, selected_paths: list[str], location: str = "work_instruction_selection") -> dict[str, object]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    missing, unknown = sorted(FIELDS - set(value)), sorted(set(value) - FIELDS)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    def strings(raw: object, field: str, allow_empty: bool) -> list[str]:
        if not isinstance(raw, list) or (not allow_empty and not raw) or any(not isinstance(item, str) or not item for item in raw):
            raise WorkError(ExitCode.CONTRACT, "invalid_string_array", "A string array with the required cardinality is required.", {"location": f"{location}.{field}"})
        return list(raw)
    stored_paths = strings(value["selected_paths"], "selected_paths", True)
    resolved_paths = strings(value["resolved_paths"], "resolved_paths", False)
    if stored_paths != selected_paths:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instruction_selection_selected_paths_mismatch", "The stored Work selected paths do not match the confirmed hierarchy selection.", {"location": f"{location}.selected_paths"})
    references = strings(value["references"], "references", True)
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Instruction reference logical names must be unique.", {"location": f"{location}.references"})
    raw_sources = value["sources"]
    if not isinstance(raw_sources, list) or not raw_sources:
        raise WorkError(ExitCode.CONTRACT, "invalid_instruction_sources", "Instruction sources must be a non-empty array.", {"location": f"{location}.sources"})
    sources: list[dict[str, str]] = []
    for index, source in enumerate(raw_sources):
        source_location = f"{location}.sources[{index}]"
        if not isinstance(source, dict) or set(source) != SOURCE_FIELDS:
            missing = sorted(SOURCE_FIELDS - set(source)) if isinstance(source, dict) else sorted(SOURCE_FIELDS)
            unknown = sorted(set(source) - SOURCE_FIELDS) if isinstance(source, dict) else []
            raise WorkError(ExitCode.CONTRACT, "invalid_object_fields" if isinstance(source, dict) else "expected_object", "The JSON object has missing or unknown fields.", {"location": source_location, "missing": missing, "unknown": unknown})
        kind, name, digest = source["kind"], source["logical_name"], source["canonical_sha256"]
        if kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_kind", "The instruction source kind is invalid.", {"location": f"{source_location}.kind", "kind": kind})
        if not isinstance(name, str) or not name:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_logical_name", "The instruction source logical name must be a non-empty string.", {"location": f"{source_location}.logical_name"})
        if not isinstance(digest, str) or __import__("re").fullmatch(SHA256_PATTERN, digest) is None:
            raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": f"{source_location}.canonical_sha256"})
        sources.append(dict(source))
    digest = value["instructions_sha256"]
    if not isinstance(digest, str) or __import__("re").fullmatch(SHA256_PATTERN, digest) is None:
        raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": f"{location}.instructions_sha256"})
    return {"selected_paths": stored_paths, "resolved_paths": resolved_paths, "sources": sources, "references": references, "instructions_sha256": digest}


def validate_work_instruction_selection(value: object, current: InstructionSourceSet, *, selected_paths: list[str], location: str = "work_instruction_selection") -> InstructionSourceSet:
    selection = parse_work_instruction_selection(value, selected_paths=selected_paths, location=location)
    if list(current.hierarchy.selected_paths) != selection["selected_paths"] or list(current.hierarchy.resolved_paths) != selection["resolved_paths"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instruction_selection_hierarchy_mismatch", "The stored Work hierarchy does not match the current mode resolution.", {"location": location})
    if list(current.references) != selection["references"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instruction_selection_references_mismatch", "The stored Work instruction references are not in actual load order.", {"location": f"{location}.references"})
    if [source.as_dict() for source in current.sources] != selection["sources"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instruction_selection_sources_mismatch", "The stored Work instruction sources do not match the current sources.", {"location": f"{location}.sources"})
    if current.instructions_sha256 != selection["instructions_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instructions_fingerprint_mismatch", "The stored Work instruction fingerprint does not match the current sources.", {"location": f"{location}.instructions_sha256"})
    return current


__all__ = ["build_work_instruction_selection", "parse_work_instruction_selection", "validate_work_instruction_selection"]
