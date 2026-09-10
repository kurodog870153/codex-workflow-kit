"""Validate stored v1 source metadata for an explicitly reviewed migration.

Content fingerprints cannot be recomputed without the old source bytes. These
checks establish metadata consistency, not authenticity of historical content.
"""

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import INSTRUCTION_SOURCE_KINDS
from .validation import SOURCE_FIELDS, sha256, strict_object, string_array


def stored_selection(value, *, selected_paths=None, document=False):
    fields = {"sources", "references", "instructions_sha256"}
    if not document:
        fields |= {"selected_paths", "resolved_paths"}
    selection = strict_object(value, location="historical instruction selection", required=fields)
    if not document:
        for key in ("selected_paths", "resolved_paths"):
            paths = string_array(selection[key], location=key, allow_empty=key == "selected_paths")
            if len(paths) != len(set(paths)):
                raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_path", "Stored paths must be unique.")
        if selected_paths is not None and selection["selected_paths"] != selected_paths:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "work_instruction_selection_selected_paths_mismatch",
                            "Stored Work paths differ from the confirmed selection.")
    references = string_array(selection["references"], location="references", allow_empty=True)
    if len(references) != len(set(references)):
        raise WorkError(ExitCode.CONTRACT, "duplicate_instruction_reference", "Stored references must be unique.")
    sha256(selection["instructions_sha256"], location="instructions_sha256")
    sources = selection["sources"]
    if not isinstance(sources, list) or not sources:
        raise WorkError(ExitCode.CONTRACT, "invalid_instruction_sources", "Stored sources must be non-empty.")
    identities = set()
    for value in sources:
        source = strict_object(value, location="source", required=SOURCE_FIELDS)
        kind, name = source["kind"], source["logical_name"]
        if not isinstance(kind, str) or kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_kind", "Invalid stored source kind.")
        if not isinstance(name, str) or not name:
            raise WorkError(ExitCode.CONTRACT, "invalid_instruction_logical_name", "Invalid stored source name.")
        sha256(source["canonical_sha256"], location="canonical_sha256")
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
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "historical_instruction_fingerprint_conflict",
                            "Identical stored source sets have different fingerprints.")
        fingerprints[signature] = fingerprint
        for source in selection["sources"]:
            identity = (source["kind"], source["logical_name"])
            if identity in identities and identities[identity] != source:
                raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "instruction_source_identity_conflict",
                                "Stored TASK sources conflict.")
            if identity not in identities:
                identities[identity] = source
                sources.append(source)
        for reference in selection["references"]:
            if reference not in references:
                references.append(reference)
    if document["sources"] != sources or document["references"] != references:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "historical_instruction_union_mismatch",
                        "Stored document sources and references must equal the TASK union.")
    signature = tuple((s["kind"], s["logical_name"], s["canonical_sha256"]) for s in sources)
    if signature in fingerprints and document["instructions_sha256"] != fingerprints[signature]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "historical_instruction_fingerprint_conflict",
                        "Stored document and TASK fingerprints conflict.")
    return document
