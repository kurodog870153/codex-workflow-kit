"""Validate stored source metadata without requiring live source resolution."""

from __future__ import annotations

import re

from ...protocol import (
    INSTRUCTION_SOURCE_KINDS,
    INVALID_SHA256_ERROR_CODE,
    SHA256_PATTERN as SHA256_PATTERN_TEXT,
)
from ...models.common.errors import ExitCode, WorkError

FIELDS = {"sources", "references", "instructions_sha256"}
SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256", "compatibility_revision"}
LEGACY_SOURCE_FIELDS = SOURCE_FIELDS - {"compatibility_revision"}
SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)


def _object(value, fields, location):
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    if set(value) != fields:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": sorted(fields - set(value)), "unknown": sorted(set(value) - fields)})
    return value


def _strings(value, location, allow_empty=True):
    if not isinstance(value, list) or (not allow_empty and not value) or any(not isinstance(item, str) or not item for item in value):
        raise WorkError(ExitCode.CONTRACT, "invalid_string_array", "A string array with the required cardinality is required.", {"location": location})
    return list(value)


def stored_selection(value, *, selected_paths=None, document=False):
    fields = set(FIELDS) if document else FIELDS | {"selected_paths", "resolved_paths"}
    selection = _object(value, fields, "historical instruction selection")
    if not document:
        for key in ("selected_paths", "resolved_paths"):
            paths = _strings(selection[key], key, key == "selected_paths")
            if len(paths) != len(set(paths)):
                raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_path", "Stored paths must be unique.")
        if selected_paths is not None and selection["selected_paths"] != selected_paths:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instruction_selection_selected_paths_mismatch", "Stored Work paths differ from the confirmed selection.")
    references = _strings(selection["references"], "references")
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Stored references must be unique.")
    if not isinstance(selection["instructions_sha256"], str) or not SHA256_PATTERN.fullmatch(selection["instructions_sha256"]):
        raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": "instructions_sha256"})
    sources = selection["sources"]
    if not isinstance(sources, list) or not sources:
        raise WorkError(ExitCode.CONTRACT, "invalid_instruction_sources", "Stored sources must be non-empty.")
    identities = set()
    for raw in sources:
        if not isinstance(raw, dict) or set(raw) not in (SOURCE_FIELDS, LEGACY_SOURCE_FIELDS):
            expected = SOURCE_FIELDS if isinstance(raw, dict) and "compatibility_revision" in raw else LEGACY_SOURCE_FIELDS
            source = _object(raw, expected, "source")
        else:
            source = raw
        kind, name, digest = source["kind"], source["logical_name"], source["canonical_sha256"]
        if not isinstance(kind, str) or kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_kind", "Invalid stored source kind.")
        if not isinstance(name, str) or not name:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_logical_name", "Invalid stored source name.")
        if not isinstance(digest, str) or not SHA256_PATTERN.fullmatch(digest):
            raise WorkError(ExitCode.CONTRACT, INVALID_SHA256_ERROR_CODE, "A SHA-256 value must contain 64 lowercase hexadecimal characters.", {"location": "canonical_sha256"})
        revision = source.get("compatibility_revision")
        if revision is not None and (not isinstance(revision, int) or isinstance(revision, bool) or revision < 1):
            raise WorkError(ExitCode.CONTRACT, "invalid_compatibility_revision", "Compatibility revision must be a positive integer.")
        if (kind, name) in identities:
            raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_source", "Stored source identities must be unique.")
        identities.add((kind, name))
    return selection


def stored_document_selection(value, task_selections):
    document = stored_selection(value, document=True)
    sources, references, identities, fingerprints = [], [], {}, {}
    for value in task_selections:
        selection = stored_selection(value)
        signature = tuple((s["kind"], s["logical_name"], s["canonical_sha256"]) for s in selection["sources"])
        fingerprint = selection["instructions_sha256"]
        if signature in fingerprints and fingerprints[signature] != fingerprint:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "historical_instruction_fingerprint_conflict", "Identical stored source sets have different fingerprints.")
        fingerprints[signature] = fingerprint
        for source in selection["sources"]:
            identity = (source["kind"], source["logical_name"])
            if identity in identities and identities[identity] != source:
                raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_source_identity_conflict", "Stored TASK sources conflict.")
            if identity not in identities:
                identities[identity] = source; sources.append(source)
        for reference in selection["references"]:
            if reference not in references:
                references.append(reference)
    if document["sources"] != sources or document["references"] != references:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "historical_instruction_union_mismatch", "Stored document sources and references must equal the TASK union.")
    signature = tuple((s["kind"], s["logical_name"], s["canonical_sha256"]) for s in sources)
    if signature in fingerprints and document["instructions_sha256"] != fingerprints[signature]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "historical_instruction_fingerprint_conflict", "Stored document and TASK fingerprints conflict.")
    return document


__all__ = ["stored_document_selection", "stored_selection"]
