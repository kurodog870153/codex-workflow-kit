from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import (
    canonical_bytes,
    canonical_sha256,
    instructions_sha256,
    read_raw,
)
from ..foundation.hierarchy import NAME_PATTERN, Hierarchy
from .catalog import resolve_instruction_hierarchy


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
    hierarchy: Hierarchy
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


def _load_source(
    resolved_skill_root: Path,
    declared_path: Path,
    *,
    kind: str,
    logical_name: str,
    resolved_boundary: Path | None = None,
) -> InstructionSource:
    try:
        resolved_path = declared_path.resolve(strict=True)
    except OSError as error:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_source_missing",
            "A required instruction source does not exist.",
            {"logical_name": logical_name, "path": str(declared_path)},
        ) from error
    if not resolved_path.is_file():
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_source_not_file",
            "A required instruction source is not a regular file.",
            {"logical_name": logical_name, "path": str(declared_path)},
        )
    if not resolved_path.is_relative_to(resolved_skill_root):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_source_escapes_skill_root",
            "An instruction source resolves outside the Work skill root.",
            {
                "logical_name": logical_name,
                "declared_path": str(declared_path),
                "resolved_path": str(resolved_path),
            },
        )
    if resolved_boundary is not None and not resolved_path.is_relative_to(
        resolved_boundary
    ):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_reference_escapes_hierarchy",
            "An instruction reference resolves outside its declaring hierarchy.",
            {
                "logical_name": logical_name,
                "declared_path": str(declared_path),
                "resolved_path": str(resolved_path),
            },
        )

    raw = read_raw(declared_path)
    canonical_content = canonical_bytes(raw, source=str(declared_path))
    return InstructionSource(
        kind=kind,
        logical_name=logical_name,
        path=declared_path,
        canonical_content=canonical_content,
        canonical_sha256=canonical_sha256(raw, source=str(declared_path)),
    )


def _route_references(
    mode: str,
    hierarchy: Hierarchy,
    reference_names: list[str],
) -> dict[str, list[tuple[str, str]]]:
    for logical_name in reference_names:
        if not isinstance(logical_name, str) or not logical_name:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_reference",
                "An instruction reference logical name must be a non-empty string.",
            )
    if len(reference_names) != len(set(reference_names)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_instruction_reference",
            "Instruction reference logical names must be unique.",
        )

    routes: dict[str, list[tuple[str, str]]] = {
        hierarchy_path: [] for hierarchy_path in hierarchy.resolved_paths
    }
    candidates = sorted(
        hierarchy.resolved_paths,
        key=lambda hierarchy_path: len(hierarchy_path.split("/")),
        reverse=True,
    )
    for logical_name in reference_names:
        for hierarchy_path in candidates:
            dotted_path = hierarchy_path.replace("/", ".")
            prefix = f"{mode}.{dotted_path}."
            if not logical_name.startswith(prefix):
                continue
            reference_name = logical_name[len(prefix) :]
            if not NAME_PATTERN.fullmatch(reference_name):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "invalid_instruction_reference",
                    "The instruction reference name must be lowercase kebab-case.",
                    {"logical_name": logical_name},
                )
            routes[hierarchy_path].append((logical_name, reference_name))
            break
        else:
            raise WorkError(
                ExitCode.CONTRACT,
                "unroutable_instruction_reference",
                "The instruction reference does not belong to a selected hierarchy.",
                {"logical_name": logical_name},
            )
    return routes


def load_instruction_sources(
    skill_root: Path,
    mode: str,
    selected_paths: list[str],
    reference_names: list[str] | None = None,
) -> InstructionSourceSet:
    hierarchy = resolve_instruction_hierarchy(skill_root, mode, selected_paths)
    routed_references = _route_references(
        mode,
        hierarchy,
        reference_names or [],
    )
    try:
        resolved_skill_root = skill_root.resolve(strict=True)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "skill_root_missing",
            "The Work skill root does not exist.",
            {"path": str(skill_root)},
        ) from error

    sources: list[InstructionSource] = [
        _load_source(
            resolved_skill_root,
            skill_root / "references" / "instruction-loading.md",
            kind="workflow",
            logical_name="work.instruction-loading",
        ),
        _load_source(
            resolved_skill_root,
            skill_root / "references" / "workflows" / f"{mode}.md",
            kind="workflow",
            logical_name=f"work.workflow.{mode}",
        ),
    ]
    for hierarchy_path in hierarchy.resolved_paths:
        hierarchy_directory = (
            skill_root
            / "references"
            / "instructions"
            / mode
            / Path(*hierarchy_path.split("/"))
        )
        sources.append(
            _load_source(
                resolved_skill_root,
                hierarchy_directory / "instructions.md",
                kind="instruction",
                logical_name=f"{mode}.{hierarchy_path.replace('/', '.')}",
            )
        )
        resolved_hierarchy_directory = hierarchy_directory.resolve(strict=True)
        for logical_name, reference_name in routed_references[hierarchy_path]:
            sources.append(
                _load_source(
                    resolved_skill_root,
                    hierarchy_directory / "references" / f"{reference_name}.md",
                    kind="reference",
                    logical_name=logical_name,
                    resolved_boundary=resolved_hierarchy_directory,
                )
            )

    source_tuple = tuple(sources)
    loaded_references = tuple(
        source.logical_name for source in source_tuple if source.kind == "reference"
    )
    fingerprint = instructions_sha256(
        mode,
        (
            (source.kind, source.logical_name, source.canonical_content)
            for source in source_tuple
        )
    )
    return InstructionSourceSet(
        mode=mode,
        hierarchy=hierarchy,
        sources=source_tuple,
        references=loaded_references,
        instructions_sha256=fingerprint,
    )
