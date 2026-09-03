from __future__ import annotations

import hashlib
import json
from pathlib import Path

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, raw_sha256, read_raw


TEXT_SUFFIXES = frozenset(
    {".bat", ".command", ".json", ".md", ".ps1", ".py", ".sh", ".toml", ".txt", ".yaml", ".yml"}
)
BUNDLE_DIRECTORIES = ("agents", "assets", "references", "scripts")
IGNORED_NAMES = frozenset({".DS_Store"})
IGNORED_SUFFIXES = frozenset({".pyc"})


def canonical_json_sha256(value: object) -> str:
    encoded = json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _is_ignored(path: Path) -> bool:
    return (
        path.name in IGNORED_NAMES
        or path.suffix.casefold() in IGNORED_SUFFIXES
        or "__pycache__" in path.parts
    )


def _is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def _bundle_paths(skill_root: Path) -> list[Path]:
    candidates = [skill_root / "SKILL.md"]
    for directory_name in BUNDLE_DIRECTORIES:
        directory = skill_root / directory_name
        if directory.is_dir():
            candidates.extend(path for path in directory.rglob("*") if path.is_file())
    return sorted(
        (path for path in candidates if not _is_ignored(path)),
        key=lambda path: path.relative_to(skill_root).as_posix().casefold(),
    )


def snapshot_skill_bundle(skill_root: Path) -> dict[str, object]:
    try:
        resolved_root = skill_root.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "skill_root_resolution_failed",
            "The skill root could not be resolved.",
            {"path": str(skill_root)},
        ) from error
    if not resolved_root.is_dir():
        raise WorkError(
            ExitCode.IO_FAILURE,
            "skill_root_not_directory",
            "The skill root is not a directory.",
            {"path": str(resolved_root)},
        )

    entries: list[dict[str, str]] = []
    framed = bytearray(b"WORK-SKILL-BUNDLE-SHA-256-V1\n")
    for path in _bundle_paths(resolved_root):
        relative_path = path.relative_to(resolved_root).as_posix()
        try:
            resolved_path = path.resolve(strict=True)
        except (OSError, RuntimeError) as error:
            raise WorkError(
                ExitCode.IO_FAILURE,
                "skill_bundle_path_resolution_failed",
                "A skill bundle path could not be resolved.",
                {"path": relative_path},
            ) from error
        if not _is_within(resolved_path, resolved_root):
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "skill_bundle_path_escapes_root",
                "A skill bundle path resolves outside the skill root.",
                {"path": relative_path, "resolved_path": str(resolved_path)},
            )
        raw = read_raw(resolved_path)
        if path.suffix.casefold() in TEXT_SUFFIXES or path.name == "SKILL.md":
            normalization = "canonical-text"
            content_sha256 = canonical_sha256(raw, source=str(resolved_path))
        else:
            normalization = "raw"
            content_sha256 = raw_sha256(raw)
        entry = {
            "path": relative_path,
            "normalization": normalization,
            "content_sha256": content_sha256,
        }
        entries.append(entry)
        framed.extend(b"F")
        for value in (relative_path, normalization, content_sha256):
            encoded = value.encode("utf-8")
            framed.extend(str(len(encoded)).encode("ascii"))
            framed.extend(b":")
            framed.extend(encoded)
        framed.extend(b"\n")
    framed.extend(b"END\n")
    return {
        "schema": "work-skill-bundle/v1",
        "files": entries,
        "bundle_sha256": hashlib.sha256(framed).hexdigest(),
    }
