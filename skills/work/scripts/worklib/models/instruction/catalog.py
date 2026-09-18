from __future__ import annotations

from dataclasses import dataclass

@dataclass(frozen=True)
class InstructionCatalog:
    mode: str
    paths: tuple[str, ...]
    children: dict[str, tuple[str, ...]]
    metadata: dict[str, dict[str, object]]
    catalog_sha256: str

    def as_dict(self) -> dict[str, object]:
        return {
            "schema": "work-instruction-catalog/v1",
            "mode": self.mode,
            "paths": list(self.paths),
            "children": {path: list(children) for path, children in self.children.items()},
            "metadata": self.metadata,
            "catalog_sha256": self.catalog_sha256,
        }


@dataclass(frozen=True)
class CrossModeInstructionCatalog:
    paths: tuple[str, ...]
    children: dict[str, tuple[str, ...]]
    metadata: dict[str, dict[str, object]]
    catalog_sha256: str

    def as_dict(self) -> dict[str, object]:
        return {
            "schema": "work-instruction-catalog/v1",
            "mode": "all",
            "paths": list(self.paths),
            "children": {path: list(children) for path, children in self.children.items()},
            "metadata": self.metadata,
            "catalog_sha256": self.catalog_sha256,
        }
