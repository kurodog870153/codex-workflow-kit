from __future__ import annotations

import re

from ...protocol import (
    INSTRUCTION_SOURCE_KINDS,
    INVALID_SHA256_ERROR_CODE,
    SHA256_PATTERN as SHA256_PATTERN_TEXT,
)
from ...technical.foundation.fingerprint import instructions_sha256
from ...models.common.errors import ExitCode, WorkError
from ...models.instruction import InstructionSource, InstructionSourceSet

FIELDS = {"sources", "references", "instructions_sha256"}
OPTIONAL_FIELDS = {"routing_manifest"}
SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256", "compatibility_revision"}
LEGACY_SOURCE_FIELDS = SOURCE_FIELDS - {"compatibility_revision"}
SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)


def build_task_document_instruction_selection(task_sources: list[InstructionSourceSet]) -> dict[str, object]:
    if not task_sources:
        raise WorkError(ExitCode.CONTRACT, "task_instruction_selections_required", "At least one TASK instruction selection is required.")
    union: list[InstructionSource] = []
    identities: dict[tuple[str, str], InstructionSource] = {}
    references: list[str] = []
    for loaded in task_sources:
        for source in loaded.sources:
            identity = (source.kind, source.logical_name)
            existing = identities.get(identity)
            if existing is not None:
                if existing.canonical_content != source.canonical_content:
                    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_source_identity_conflict", "One instruction source identity resolved to different content.", {"kind": source.kind, "logical_name": source.logical_name})
                continue
            identities[identity] = source
            union.append(source)
        for reference in loaded.references:
            if reference not in references:
                references.append(reference)
    fingerprint = instructions_sha256("task", ((source.kind, source.logical_name, source.canonical_content) for source in union))
    return {"sources": [source.as_dict() for source in union], "references": references, "instructions_sha256": fingerprint}


def validate_task_document_instruction_selection(value: object, expected: dict[str, object], *, location: str = "instruction_selection") -> dict[str, object]:
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    missing, unknown = sorted(FIELDS - set(value)), sorted(set(value) - FIELDS - OPTIONAL_FIELDS)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    if "routing_manifest" in value:
        from ...models.workflow import InstructionSelectionManifestContract
        InstructionSelectionManifestContract.model_validate(value["routing_manifest"])
    raw_sources = value["sources"]
    if not isinstance(raw_sources, list) or not raw_sources:
        raise WorkError(ExitCode.CONTRACT, "invalid_instruction_sources", "Document instruction sources must be a non-empty array.", {"location": f"{location}.sources"})
    sources: list[dict[str, str]] = []
    identities: set[tuple[str, str]] = set()
    for index, source in enumerate(raw_sources):
        source_location = f"{location}.sources[{index}]"
        if not isinstance(source, dict) or set(source) not in (SOURCE_FIELDS, LEGACY_SOURCE_FIELDS):
            expected = SOURCE_FIELDS if isinstance(source, dict) and "compatibility_revision" in source else LEGACY_SOURCE_FIELDS
            details = {"location": source_location}
            if isinstance(source, dict):
                details.update({
                    "missing": sorted(expected - set(source)),
                    "unknown": sorted(set(source) - expected),
                })
            raise WorkError(ExitCode.CONTRACT, "invalid_object_fields" if isinstance(source, dict) else "expected_object", "The JSON object has missing or unknown fields.", details)
        kind, name, digest = source["kind"], source["logical_name"], source["canonical_sha256"]
        if kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_kind", "The instruction source kind is invalid.", {"location": f"{source_location}.kind"})
        if not isinstance(name, str) or not name:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_logical_name", "The instruction source logical name must be non-empty.", {"location": f"{source_location}.logical_name"})
        if not isinstance(digest, str) or not SHA256_PATTERN.fullmatch(digest):
            raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": f"{source_location}.canonical_sha256"})
        revision = source.get("compatibility_revision")
        if revision is not None and (not isinstance(revision, int) or isinstance(revision, bool) or revision < 1):
            raise WorkError(ExitCode.CONTRACT, "invalid_compatibility_revision", "Compatibility revision must be a positive integer.", {"location": f"{source_location}.compatibility_revision"})
        if (kind, name) in identities:
            raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_source", "Document instruction source identities must be unique.", {"location": source_location})
        identities.add((kind, name)); sources.append(dict(source))
    references = value["references"]
    if not isinstance(references, list) or any(not isinstance(item, str) or not item for item in references):
        raise WorkError(ExitCode.CONTRACT, "invalid_string_array", "Document instruction references must be a string array.", {"location": f"{location}.references"})
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Document instruction references must be unique.", {"location": f"{location}.references"})
    fingerprint = value["instructions_sha256"]
    if not isinstance(fingerprint, str) or not SHA256_PATTERN.fullmatch(fingerprint):
        raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": f"{location}.instructions_sha256"})
    if sources != expected["sources"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_document_instruction_sources_mismatch", "Document instruction sources do not match the TASK source union.", {"location": f"{location}.sources"})
    if references != expected["references"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_document_instruction_references_mismatch", "Document instruction references do not match the TASK reference union.", {"location": f"{location}.references"})
    if fingerprint != expected["instructions_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_document_instructions_fingerprint_mismatch", "The document instruction fingerprint does not match its source union.", {"location": f"{location}.instructions_sha256"})
    return expected


__all__ = ["build_task_document_instruction_selection", "validate_task_document_instruction_selection"]
