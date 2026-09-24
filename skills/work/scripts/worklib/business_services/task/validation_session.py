"""Invocation-local instruction snapshots for TASK validation."""

from pathlib import Path

from ...services.instruction.catalog import build_instruction_catalog
from ...services.instruction.source import load_instruction_sources
from ...services.instruction.validation_session import ValidationSession as _ValidationSession


class ValidationSession(_ValidationSession):
    def __init__(self, skill_root: Path) -> None:
        super().__init__(skill_root, build_catalog=build_instruction_catalog,
                         load_sources=load_instruction_sources)

__all__ = ["ValidationSession"]
