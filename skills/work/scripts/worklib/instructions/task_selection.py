from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import INSTRUCTION_SOURCE_KINDS, instructions_sha256
from .selection import validate_instruction_selection
from .sources import InstructionSource


SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
DOCUMENT_FIELDS = {"sources", "references", "instructions_sha256"}
SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256"}


def _strict_object(
    value: object,
    *,
    location: str,
    fields: set[str],
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


def build_task_document_instruction_selection(
    task_selections: list[object],
    *,
    skill_root: Path,
) -> dict[str, object]:
    if not isinstance(task_selections, list) or not task_selections:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_instruction_selections_required",
            "At least one TASK instruction selection is required.",
        )

    union_sources: list[InstructionSource] = []
    source_by_identity: dict[tuple[str, str], InstructionSource] = {}
    references: list[str] = []
    for index, selection in enumerate(task_selections):
        loaded = validate_instruction_selection(
            selection,
            skill_root=skill_root,
            mode="task",
            location=f"tasks[{index}].instruction_selection",
        )
        for source in loaded.sources:
            identity = (source.kind, source.logical_name)
            existing = source_by_identity.get(identity)
            if existing is not None:
                if existing.canonical_content != source.canonical_content:
                    raise WorkError(
                        ExitCode.ARTIFACT_INTEGRITY,
                        "instruction_source_identity_conflict",
                        "One instruction source identity resolved to different content.",
                        {"kind": source.kind, "logical_name": source.logical_name},
                    )
                continue
            source_by_identity[identity] = source
            union_sources.append(source)
        for reference in loaded.references:
            if reference not in references:
                references.append(reference)

    fingerprint = instructions_sha256(
        "task",
        (
            (source.kind, source.logical_name, source.canonical_content)
            for source in union_sources
        ),
    )
    return {
        "sources": [source.as_dict() for source in union_sources],
        "references": references,
        "instructions_sha256": fingerprint,
    }


def validate_task_document_instruction_selection(
    value: object,
    task_selections: list[object],
    *,
    skill_root: Path,
    location: str = "instruction_selection",
) -> dict[str, object]:
    selection = _strict_object(
        value,
        location=location,
        fields=DOCUMENT_FIELDS,
    )
    raw_sources = selection["sources"]
    if not isinstance(raw_sources, list) or not raw_sources:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_instruction_sources",
            "Document instruction sources must be a non-empty array.",
            {"location": f"{location}.sources"},
        )
    sources: list[dict[str, str]] = []
    identities: set[tuple[str, str]] = set()
    for index, raw_source in enumerate(raw_sources):
        source_location = f"{location}.sources[{index}]"
        source = _strict_object(
            raw_source,
            location=source_location,
            fields=SOURCE_FIELDS,
        )
        kind = source["kind"]
        logical_name = source["logical_name"]
        canonical_hash = source["canonical_sha256"]
        if kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_kind",
                "The instruction source kind is invalid.",
                {"location": f"{source_location}.kind"},
            )
        if not isinstance(logical_name, str) or not logical_name:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_logical_name",
                "The instruction source logical name must be non-empty.",
                {"location": f"{source_location}.logical_name"},
            )
        if not isinstance(canonical_hash, str) or not SHA256_PATTERN.fullmatch(
            canonical_hash
        ):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_sha256",
                "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
                {"location": f"{source_location}.canonical_sha256"},
            )
        identity = (kind, logical_name)
        if identity in identities:
            raise WorkError(
                ExitCode.CONTRACT,
                "duplicate_instruction_source",
                "Document instruction source identities must be unique.",
                {"location": source_location},
            )
        identities.add(identity)
        sources.append(
            {
                "kind": kind,
                "logical_name": logical_name,
                "canonical_sha256": canonical_hash,
            }
        )

    raw_references = selection["references"]
    if not isinstance(raw_references, list) or any(
        not isinstance(reference, str) or not reference
        for reference in raw_references
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "Document instruction references must be a string array.",
            {"location": f"{location}.references"},
        )
    references = list(raw_references)
    if len(references) != len(set(references)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_instruction_reference",
            "Document instruction references must be unique.",
            {"location": f"{location}.references"},
        )
    stored_fingerprint = selection["instructions_sha256"]
    if not isinstance(stored_fingerprint, str) or not SHA256_PATTERN.fullmatch(
        stored_fingerprint
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            {"location": f"{location}.instructions_sha256"},
        )

    expected = build_task_document_instruction_selection(
        task_selections,
        skill_root=skill_root,
    )
    if sources != expected["sources"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_document_instruction_sources_mismatch",
            "Document instruction sources do not match the TASK source union.",
            {"location": f"{location}.sources"},
        )
    if references != expected["references"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_document_instruction_references_mismatch",
            "Document instruction references do not match the TASK reference union.",
            {"location": f"{location}.references"},
        )
    if stored_fingerprint != expected["instructions_sha256"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_document_instructions_fingerprint_mismatch",
            "The document instruction fingerprint does not match its source union.",
            {"location": f"{location}.instructions_sha256"},
        )
    return expected
