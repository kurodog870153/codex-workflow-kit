"""Invocation-local instruction snapshots and authoritative source rechecks."""

from pathlib import Path
from typing import Callable

from ...models.common.errors import ExitCode, WorkError
from ...models.hierarchy import HierarchyContract
from ...models.instruction import InstructionCatalog, InstructionSourceSet


class ValidationSession:
    def __init__(self, skill_root: Path, *,
                 build_catalog: Callable[[Path, str], InstructionCatalog],
                 load_sources: Callable[..., InstructionSourceSet]) -> None:
        self.skill_root = skill_root
        self._build_catalog = build_catalog
        self._load_sources = load_sources
        self._catalogs: dict[str, InstructionCatalog] = {}
        self._sources: dict[tuple[object, ...], tuple[HierarchyContract, list[str], InstructionSourceSet]] = {}

    def catalog(self, mode: str) -> InstructionCatalog:
        if mode not in self._catalogs:
            self._catalogs[mode] = self._build_catalog(self.skill_root, mode)
        return self._catalogs[mode]

    def sources(self, mode: str, hierarchy: HierarchyContract,
                references: list[str]) -> InstructionSourceSet:
        key = (mode, hierarchy.selected_paths, hierarchy.resolved_paths, tuple(references))
        if key not in self._sources:
            self._sources[key] = (
                hierarchy, list(references),
                self._load_sources(self.skill_root, mode, hierarchy, references),
            )
        return self._sources[key][2]

    def recheck(self) -> None:
        for mode, catalog in self._catalogs.items():
            if self._build_catalog(self.skill_root, mode).catalog_sha256 != catalog.catalog_sha256:
                raise WorkError(
                    ExitCode.ARTIFACT_INTEGRITY, "instruction_catalog_changed",
                    "The instruction catalog changed during TASK validation.",
                )
        for key, (hierarchy, references, sources) in self._sources.items():
            current = self._load_sources(self.skill_root, str(key[0]), hierarchy, references)
            if current.instructions_sha256 != sources.instructions_sha256:
                raise WorkError(
                    ExitCode.ARTIFACT_INTEGRITY, "instruction_source_changed",
                    "An instruction source changed during TASK validation.",
                )
