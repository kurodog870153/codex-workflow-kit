from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from ..hierarchy import HierarchyContract


@dataclass(frozen=True)
class InstructionSource:
    kind: str
    logical_name: str
    path: Path
    canonical_content: bytes
    canonical_sha256: str

    def as_dict(self) -> dict[str, str]:
        return {
            "kind": self.kind,
            "logical_name": self.logical_name,
            "canonical_sha256": self.canonical_sha256,
        }


@dataclass(frozen=True)
class InstructionSourceSet:
    mode: str
    hierarchy: HierarchyContract
    sources: tuple[InstructionSource, ...]
    references: tuple[str, ...]
    instructions_sha256: str

    def as_dict(self) -> dict[str, object]:
        return {
            "schema": "work-instructions/v1",
            "mode": self.mode,
            "hierarchy": self.hierarchy.as_dict(),
            "sources": [source.as_dict() for source in self.sources],
            "references": list(self.references),
            "instructions_sha256": self.instructions_sha256,
        }
