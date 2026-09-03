from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import decode_utf8, read_raw
from ..foundation.hierarchy import (
    NAME_PATTERN,
    WORK_DIRECTORIES,
    Hierarchy,
    build_hierarchy,
)
from ..skills.fingerprint import canonical_json_sha256


MODES = ("plan", "task", "execute")


@dataclass(frozen=True)
class InstructionCatalog:
    mode: str
    paths: tuple[str, ...]
    children: dict[str, tuple[str, ...]]
    metadata: dict[str, dict[str, object]]

    def as_dict(self) -> dict[str, object]:
        catalog: dict[str, object] = {
            "schema": "work-instruction-catalog/v1",
            "mode": self.mode,
            "paths": list(self.paths),
            "children": {
                path: list(children) for path, children in self.children.items()
            },
            "metadata": self.metadata,
        }
        catalog["catalog_sha256"] = canonical_json_sha256(catalog)
        return catalog


@dataclass(frozen=True)
class CrossModeInstructionCatalog:
    paths: tuple[str, ...]
    children: dict[str, tuple[str, ...]]
    metadata: dict[str, dict[str, object]]

    def as_dict(self) -> dict[str, object]:
        catalog: dict[str, object] = {
            "schema": "work-instruction-catalog/v1",
            "mode": "all",
            "paths": list(self.paths),
            "children": {
                path: list(children) for path, children in self.children.items()
            },
            "metadata": self.metadata,
        }
        catalog["catalog_sha256"] = canonical_json_sha256(catalog)
        return catalog


FRONTMATTER_FIELDS = frozenset({"name", "description", "metadata"})
METADATA_FIELDS = frozenset({"work-tags"})


def _instruction_metadata(path: Path) -> dict[str, object]:
    source = str(path)
    text = decode_utf8(read_raw(path), source=source)
    lines = text.splitlines()
    if not lines or lines[0] != "---":
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "instruction_frontmatter_missing",
            "instructions.md must begin with YAML frontmatter.",
            {"source": source},
        )
    try:
        boundary = lines.index("---", 1)
    except ValueError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "instruction_frontmatter_unterminated",
            "instructions.md YAML frontmatter is not terminated.",
            {"source": source},
        ) from error
    try:
        loaded: Any = yaml.safe_load("\n".join(lines[1:boundary]))
    except yaml.YAMLError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_instruction_frontmatter",
            "instructions.md frontmatter is not valid YAML.",
            {"source": source},
        ) from error
    if not isinstance(loaded, dict) or any(
        not isinstance(key, str) for key in loaded
    ):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_instruction_frontmatter",
            "instructions.md frontmatter must be a YAML object with string keys.",
            {"source": source},
        )
    missing = sorted(FRONTMATTER_FIELDS - set(loaded))
    unknown = sorted(set(loaded) - FRONTMATTER_FIELDS)
    if missing or unknown:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_instruction_metadata_fields",
            "Instruction frontmatter has missing or unknown fields.",
            {"source": source, "missing": missing, "unknown": unknown},
        )

    result: dict[str, object] = {}
    for field in ("name", "description"):
        value = loaded[field]
        if not isinstance(value, str) or not value.strip():
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "invalid_instruction_metadata_value",
                "Instruction name and description must be non-empty strings.",
                {"source": source, "field": field},
            )
        result[field] = value.strip()

    metadata = loaded["metadata"]
    if not isinstance(metadata, dict) or any(
        not isinstance(key, str) for key in metadata
    ):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_instruction_metadata",
            "Instruction metadata must be a YAML object with string keys.",
            {"source": source},
        )
    missing = sorted(METADATA_FIELDS - set(metadata))
    unknown = sorted(set(metadata) - METADATA_FIELDS)
    if missing or unknown:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_instruction_metadata_fields",
            "Instruction metadata has missing or unknown fields.",
            {"source": source, "missing": missing, "unknown": unknown},
        )
    tags = metadata["work-tags"]
    if (
        not isinstance(tags, list)
        or not tags
        or any(not isinstance(tag, str) or not NAME_PATTERN.fullmatch(tag) for tag in tags)
        or len(tags) != len(set(tags))
    ):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_instruction_work_tags",
            "Instruction work-tags must be a non-empty array of unique lowercase kebab-case strings.",
            {"source": source},
        )
    result["work_tags"] = list(tags)
    return result


def _validate_mode(mode: str) -> None:
    if mode not in WORK_DIRECTORIES:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_instruction_mode",
            "The instruction mode must be plan, task, or execute.",
            {"mode": mode},
        )


def _resolve_directory(path: Path, code: str, message: str) -> Path:
    try:
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            code,
            message,
            {"path": str(path), "reason": str(error)},
        ) from error
    if not resolved.is_dir():
        raise WorkError(
            ExitCode.IO_FAILURE,
            code,
            message,
            {"path": str(path)},
        )
    return resolved


def build_instruction_catalog(skill_root: Path, mode: str) -> InstructionCatalog:
    _validate_mode(mode)
    resolved_skill_root = _resolve_directory(
        skill_root,
        "skill_root_missing",
        "The Work skill root does not exist or is not a directory.",
    )
    instruction_root = skill_root / "references" / "instructions" / mode
    resolved_instruction_root = _resolve_directory(
        instruction_root,
        "instruction_root_missing",
        "The instruction mode directory does not exist or is not a directory.",
    )
    if not resolved_instruction_root.is_relative_to(resolved_skill_root):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "instruction_root_escapes_skill_root",
            "The instruction mode directory resolves outside the Work skill root.",
            {"mode": mode, "path": str(instruction_root)},
        )

    catalog_paths: list[str] = []
    catalog_metadata: dict[str, dict[str, object]] = {}
    for entrypoint in instruction_root.rglob("instructions.md"):
        try:
            resolved_entrypoint = entrypoint.resolve(strict=True)
        except OSError as error:
            raise WorkError(
                ExitCode.IO_FAILURE,
                "instruction_entrypoint_unreadable",
                "An instruction entrypoint cannot be resolved.",
                {"path": str(entrypoint), "reason": str(error)},
            ) from error
        if (
            not resolved_entrypoint.is_file()
            or not resolved_entrypoint.is_relative_to(resolved_skill_root)
            or not resolved_entrypoint.is_relative_to(resolved_instruction_root)
        ):
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "instruction_path_escapes_skill_root",
                "An instruction entrypoint resolves outside its instruction directory.",
                {"mode": mode, "path": str(entrypoint)},
            )

        relative_directory = entrypoint.parent.relative_to(instruction_root)
        parts = relative_directory.parts
        if not parts or any(not NAME_PATTERN.fullmatch(part) for part in parts):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_hierarchy_path",
                "An instruction hierarchy path is invalid.",
                {"mode": mode, "path": relative_directory.as_posix()},
            )
        hierarchy_path = "/".join(parts)
        if "general" in parts and hierarchy_path != "general":
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_general_instruction_path",
                "The general instruction entrypoint must be a root hierarchy path.",
                {"mode": mode, "path": hierarchy_path},
            )
        catalog_paths.append(hierarchy_path)
        catalog_metadata[hierarchy_path] = _instruction_metadata(entrypoint)

    if catalog_paths.count("general") != 1:
        raise WorkError(
            ExitCode.CONTRACT,
            "general_instruction_not_unique",
            "The instruction mode must contain exactly one general entrypoint.",
            {"mode": mode, "count": catalog_paths.count("general")},
        )

    path_set = set(catalog_paths)
    for hierarchy_path in path_set - {"general"}:
        parts = hierarchy_path.split("/")
        for depth in range(1, len(parts)):
            ancestor = "/".join(parts[:depth])
            if ancestor not in path_set:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "instruction_ancestor_missing",
                    "Every instruction hierarchy path must have all ancestor entrypoints.",
                    {
                        "mode": mode,
                        "path": hierarchy_path,
                        "missing_ancestor": ancestor,
                    },
                )

    paths = tuple(sorted(path_set, key=lambda path: (path != "general", path)))
    mutable_children: dict[str, list[str]] = {path: [] for path in paths}
    for hierarchy_path in paths:
        if hierarchy_path == "general":
            continue
        parent, separator, child = hierarchy_path.rpartition("/")
        parent_path = parent if separator else "general"
        mutable_children[parent_path].append(child)
    children = {
        path: tuple(sorted(child_names))
        for path, child_names in mutable_children.items()
    }
    metadata = {path: catalog_metadata[path] for path in paths}
    return InstructionCatalog(
        mode=mode,
        paths=paths,
        children=children,
        metadata=metadata,
    )


def build_cross_mode_instruction_catalog(
    skill_root: Path,
) -> CrossModeInstructionCatalog:
    catalogs = {mode: build_instruction_catalog(skill_root, mode) for mode in MODES}
    path_set = {
        hierarchy_path
        for catalog in catalogs.values()
        for hierarchy_path in catalog.paths
    }
    paths = tuple(sorted(path_set, key=lambda path: (path != "general", path)))

    mutable_children: dict[str, set[str]] = {path: set() for path in paths}
    for hierarchy_path in paths:
        if hierarchy_path == "general":
            continue
        parent, separator, child = hierarchy_path.rpartition("/")
        parent_path = parent if separator else "general"
        mutable_children[parent_path].add(child)
    children = {
        path: tuple(sorted(child_names))
        for path, child_names in mutable_children.items()
    }

    metadata: dict[str, dict[str, object]] = {}
    for hierarchy_path in paths:
        mode_support = [
            mode for mode in MODES if hierarchy_path in catalogs[mode].metadata
        ]
        metadata[hierarchy_path] = {
            "mode_support": mode_support,
            "modes": {
                mode: catalogs[mode].metadata[hierarchy_path]
                for mode in mode_support
            },
        }
    return CrossModeInstructionCatalog(
        paths=paths,
        children=children,
        metadata=metadata,
    )


def resolve_instruction_hierarchy(
    skill_root: Path,
    mode: str,
    selected_paths: list[str],
) -> Hierarchy:
    catalog = build_instruction_catalog(skill_root, mode)
    hierarchy = build_hierarchy(mode, selected_paths)
    if mode == "plan":
        if not hierarchy.selected_paths:
            return hierarchy
        cross_mode_catalog = build_cross_mode_instruction_catalog(skill_root)
        cross_mode_paths = set(cross_mode_catalog.paths)
        for hierarchy_path in hierarchy.selected_paths:
            if hierarchy_path in cross_mode_paths:
                continue
            parent, separator, _ = hierarchy_path.rpartition("/")
            parent_path = parent if separator else "general"
            raise WorkError(
                ExitCode.CONTRACT,
                "instruction_hierarchy_path_missing",
                "A selected instruction hierarchy path does not exist in the catalog.",
                {
                    "mode": "all",
                    "path": hierarchy_path,
                    "parent": parent_path,
                    "valid_choices": list(
                        cross_mode_catalog.children.get(parent_path, ())
                    ),
                },
            )

        mode_paths = set(catalog.paths)
        projected_candidates: list[str] = []
        for selected_path in hierarchy.selected_paths:
            parts = selected_path.split("/")
            projected_path = next(
                (
                    "/".join(parts[:depth])
                    for depth in range(len(parts), 0, -1)
                    if "/".join(parts[:depth]) in mode_paths
                ),
                None,
            )
            if projected_path is not None and projected_path not in projected_candidates:
                projected_candidates.append(projected_path)
        projected_paths = [
            path
            for path in projected_candidates
            if not any(
                candidate.startswith(f"{path}/")
                for candidate in projected_candidates
            )
        ]
        projected = build_hierarchy(mode, projected_paths)
        return Hierarchy(
            work_directory=mode,
            selected_paths=hierarchy.selected_paths,
            resolved_paths=projected.resolved_paths,
            required_paths=projected.required_paths,
            optional_paths=projected.optional_paths,
        )

    catalog_paths = set(catalog.paths)
    for hierarchy_path in hierarchy.resolved_paths:
        if hierarchy_path in catalog_paths:
            continue
        parent, separator, _ = hierarchy_path.rpartition("/")
        parent_path = parent if separator else "general"
        raise WorkError(
            ExitCode.CONTRACT,
            "instruction_hierarchy_path_missing",
            "A selected instruction hierarchy path does not exist in the catalog.",
            {
                "mode": mode,
                "path": hierarchy_path,
                "parent": parent_path,
                "valid_choices": list(catalog.children.get(parent_path, ())),
            },
        )
    return hierarchy
