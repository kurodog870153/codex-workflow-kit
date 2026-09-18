from __future__ import annotations

from ...models.instruction import InstructionSourceSet


def build_instruction_selection(loaded: InstructionSourceSet) -> dict[str, object]:
    return {
        "selected_paths": list(loaded.hierarchy.selected_paths),
        "resolved_paths": list(loaded.hierarchy.resolved_paths),
        "sources": [source.as_dict() for source in loaded.sources],
        "references": list(loaded.references),
        "instructions_sha256": loaded.instructions_sha256,
    }


__all__ = ["build_instruction_selection"]
