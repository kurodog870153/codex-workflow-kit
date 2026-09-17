from __future__ import annotations

from pathlib import Path

from ..contracts.instruction import (
    InstructionCatalogContract,
    InstructionSelectionContract,
    InstructionsContract,
)
from .instruction_catalog import (
    build_cross_mode_instruction_catalog,
    build_instruction_catalog,
    resolve_instruction_hierarchy,
)
from .instruction_selection import build_instruction_selection
from .instruction_sources import load_instruction_sources


def catalog(skill_root: Path, mode: str) -> dict[str, object]:
    value = (
        build_cross_mode_instruction_catalog(skill_root).as_dict()
        if mode == "all" else build_instruction_catalog(skill_root, mode).as_dict()
    )
    return InstructionCatalogContract.model_validate(value).to_canonical_dict()


def resolve(skill_root: Path, mode: str, paths: list[str]) -> dict[str, object]:
    return resolve_instruction_hierarchy(skill_root, mode, paths).as_dict()


def load(skill_root: Path, mode: str, paths: list[str], references: list[str]) -> dict[str, object]:
    value = load_instruction_sources(skill_root, mode, paths, references).as_dict()
    return InstructionsContract.model_validate(value).to_canonical_dict()


def select(skill_root: Path, mode: str, paths: list[str], references: list[str]) -> dict[str, object]:
    value = {
        "schema": "work-instruction-selection/v1", "mode": mode,
        "instruction_selection": build_instruction_selection(
            skill_root=skill_root, mode=mode, selected_paths=paths,
            reference_names=references,
        ),
    }
    return InstructionSelectionContract.model_validate(value).to_canonical_dict()
