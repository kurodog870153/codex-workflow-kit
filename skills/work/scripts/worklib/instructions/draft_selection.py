"""Structured, ordered instruction choices for saved TASK planning."""

from __future__ import annotations

from ..contracts.validation import strict_keys
from ..foundation.errors import ExitCode, WorkError


def validate_draft_instruction_selection(value: object) -> dict[str, list[str]]:
    """Validate storage shape only; live catalogs and fingerprints are separate."""
    selection = strict_keys(value, location="instruction_selection", required={"selected_paths", "references"})
    for values in selection.values():
        if not isinstance(values, list) or any(not isinstance(item, str) or not item.strip() for item in values) or len(values) != len(set(values)):
            raise WorkError(ExitCode.CONTRACT, "invalid_source_selection", "Instruction selections must be unique string arrays.")
    return {field: list(values) for field, values in selection.items()}


def resolve_draft_instruction_selection(
    entry: dict[str, object], *, selected_paths: list[str] | None = None,
    reference_names: list[str] | None = None,
) -> dict[str, list[str]]:
    """Use stored choices or an explicit legacy choice, never infer from notes."""
    stored = validate_draft_instruction_selection(entry["instruction_selection"]) if "instruction_selection" in entry else None
    if selected_paths is None:
        if reference_names is not None:
            raise WorkError(ExitCode.CONTRACT, "draft_selection_incomplete", "Explicit references require an explicit instruction path selection.")
        if stored is None:
            raise WorkError(ExitCode.WORKFLOW_STATE, "draft_selection_required", "Confirm instruction paths and references for this legacy TASK before continuing.")
        return stored
    explicit = validate_draft_instruction_selection({"selected_paths": selected_paths, "references": [] if reference_names is None else reference_names})
    if stored is not None and explicit != stored:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_selection_mismatch", "Changing a saved instruction selection requires the source-update workflow.")
    return explicit
