from __future__ import annotations

from pathlib import Path

from ..foundation.errors import ExitCode, WorkError
from .selection import SOURCE_FIELDS, _sha256, _strict_object, _string_array
from .sources import InstructionSourceSet, load_instruction_sources
from ..foundation.fingerprint import INSTRUCTION_SOURCE_KINDS


SELECTION_FIELDS = {
    "selected_paths",
    "resolved_paths",
    "sources",
    "references",
    "instructions_sha256",
}


def build_work_instruction_selection(
    *,
    skill_root: Path,
    mode: str,
    selected_paths: list[str],
    reference_names: list[str] | None = None,
) -> dict[str, object]:
    loaded = load_instruction_sources(
        skill_root,
        mode,
        selected_paths=selected_paths,
        reference_names=reference_names,
    )
    return {
        "selected_paths": list(loaded.hierarchy.selected_paths),
        "resolved_paths": list(loaded.hierarchy.resolved_paths),
        "sources": [source.as_dict() for source in loaded.sources],
        "references": list(loaded.references),
        "instructions_sha256": loaded.instructions_sha256,
    }


def validate_work_instruction_selection(
    value: object,
    *,
    skill_root: Path,
    mode: str,
    selected_paths: list[str],
    location: str = "work_instruction_selection",
) -> InstructionSourceSet:
    selection = _strict_object(value, location=location, required=SELECTION_FIELDS)
    stored_selected_paths = _string_array(
        selection["selected_paths"],
        location=f"{location}.selected_paths",
        allow_empty=True,
    )
    stored_resolved_paths = _string_array(
        selection["resolved_paths"],
        location=f"{location}.resolved_paths",
        allow_empty=False,
    )
    if stored_selected_paths != selected_paths:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "work_instruction_selection_selected_paths_mismatch",
            "The stored Work selected paths do not match the confirmed hierarchy selection.",
            {"location": f"{location}.selected_paths"},
        )
    references = _string_array(
        selection["references"], location=f"{location}.references", allow_empty=True
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
        source = _strict_object(raw_source, location=source_location, required=SOURCE_FIELDS)
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
        sources.append(
            {
                "kind": kind,
                "logical_name": logical_name,
                "canonical_sha256": _sha256(
                    source["canonical_sha256"],
                    location=f"{source_location}.canonical_sha256",
                ),
            }
        )
    stored_fingerprint = _sha256(
        selection["instructions_sha256"],
        location=f"{location}.instructions_sha256",
    )
    current = load_instruction_sources(
        skill_root,
        mode,
        selected_paths=stored_selected_paths,
        reference_names=references,
    )
    if (
        list(current.hierarchy.selected_paths) != stored_selected_paths
        or list(current.hierarchy.resolved_paths) != stored_resolved_paths
    ):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "work_instruction_selection_hierarchy_mismatch",
            "The stored Work hierarchy does not match the current mode resolution.",
            {"location": location},
        )
    if list(current.references) != references:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "work_instruction_selection_references_mismatch",
            "The stored Work instruction references are not in actual load order.",
            {"location": f"{location}.references"},
        )
    if [source.as_dict() for source in current.sources] != sources:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "work_instruction_selection_sources_mismatch",
            "The stored Work instruction sources do not match the current sources.",
            {"location": f"{location}.sources"},
        )
    if current.instructions_sha256 != stored_fingerprint:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "work_instructions_fingerprint_mismatch",
            "The stored Work instruction fingerprint does not match the current sources.",
            {"location": f"{location}.instructions_sha256"},
        )
    return current
