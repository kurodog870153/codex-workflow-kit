from __future__ import annotations

import hashlib
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any

import yaml

from ..foundation.errors import ExitCode, WorkError
from .fingerprint import canonical_json_sha256, snapshot_skill_bundle


VALID_SCOPES = frozenset({"repo", "user", "admin", "system"})
VALID_MODES = frozenset({"plan", "task", "execute"})


@dataclass(frozen=True)
class SkillRoot:
    scope: str
    locator: str
    path: Path


def _strict_mapping(value: object, *, source: str) -> dict[str, Any]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_yaml_object",
            "Skill YAML must decode to an object with string keys.",
            {"source": source},
        )
    return value


def _frontmatter(path: Path) -> dict[str, Any]:
    source = str(path)
    try:
        stream = path.open("rb")
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "skill_summary_read_failed",
            "The skill summary could not be read.",
            {"source": source},
        ) from error
    with stream:
        first = stream.readline()
        if first.startswith(b"\xef\xbb\xbf"):
            first = first[3:]
        if first.rstrip(b"\r\n") != b"---":
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "skill_frontmatter_missing",
                "SKILL.md must begin with YAML frontmatter.",
                {"source": source},
            )
        lines: list[bytes] = []
        while True:
            line = stream.readline()
            if line == b"":
                raise WorkError(
                    ExitCode.INPUT_FORMAT,
                    "skill_frontmatter_unterminated",
                    "SKILL.md YAML frontmatter is not terminated.",
                    {"source": source},
                )
            if line.rstrip(b"\r\n") == b"---":
                break
            lines.append(line)
    try:
        text = b"".join(lines).decode("utf-8", errors="strict")
        loaded = yaml.safe_load(text)
    except (UnicodeDecodeError, yaml.YAMLError) as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_frontmatter",
            "SKILL.md frontmatter is not valid UTF-8 YAML.",
            {"source": source},
        ) from error
    return _strict_mapping(loaded, source=source)


def _yaml_file(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8-sig", newline=None) as stream:
            loaded = yaml.safe_load(stream)
    except (OSError, UnicodeError, yaml.YAMLError) as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_metadata",
            "The skill metadata file is not valid UTF-8 YAML.",
            {"source": str(path)},
        ) from error
    return _strict_mapping(loaded, source=str(path))


def _string_list(value: object, *, field: str, allowed: frozenset[str] | None = None) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        items = [item.strip() for item in value.split(",") if item.strip()]
    elif isinstance(value, list) and all(isinstance(item, str) for item in value):
        items = [item.strip() for item in value if item.strip()]
    else:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_metadata_field",
            "The skill metadata field must be a string or string array.",
            {"field": field},
        )
    if len(items) != len(set(items)) or (allowed is not None and any(item not in allowed for item in items)):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_metadata_value",
            "The skill metadata field contains duplicate or unsupported values.",
            {"field": field, "values": items},
        )
    return items


def _normalize_dependencies(value: object, *, source: str) -> list[dict[str, object]]:
    if value is None:
        return []
    dependencies = _strict_mapping(value, source=source)
    tools = dependencies.get("tools", [])
    if not isinstance(tools, list):
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_dependencies",
            "Skill tool dependencies must be an array.",
            {"source": source},
        )
    result: list[dict[str, object]] = []
    for index, raw_tool in enumerate(tools):
        tool = _strict_mapping(raw_tool, source=f"{source}:dependencies.tools[{index}]")
        if not isinstance(tool.get("type"), str) or not isinstance(tool.get("value"), str):
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "invalid_skill_dependency",
                "Each skill tool dependency requires string type and value fields.",
                {"source": source, "index": index},
            )
        normalized: dict[str, object] = {"type": tool["type"], "value": tool["value"]}
        for field in ("description", "transport", "url"):
            if field in tool:
                if not isinstance(tool[field], str):
                    raise WorkError(
                        ExitCode.INPUT_FORMAT,
                        "invalid_skill_dependency",
                        "Known skill dependency fields must be strings.",
                        {"source": source, "index": index, "field": field},
                    )
                normalized[field] = tool[field]
        result.append(normalized)
    return result


def _identity(name: str, scope: str, root_locator: str, source: str) -> str:
    framed = (
        f"WORK-SKILL-IDENTITY-V1\n{name}\n{scope}\n{root_locator}\n{source}\n"
    ).encode("utf-8")
    return hashlib.sha256(framed).hexdigest()


def read_skill_summary(
    skill_root: Path,
    *,
    scope: str,
    root_locator: str,
    source: str,
) -> dict[str, object]:
    frontmatter = _frontmatter(skill_root / "SKILL.md")
    name = frontmatter.get("name")
    description = frontmatter.get("description")
    if not isinstance(name, str) or not name.strip() or not isinstance(description, str) or not description.strip():
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_skill_identity",
            "Skill frontmatter requires non-empty name and description strings.",
            {"source": source},
        )
    metadata = frontmatter.get("metadata", {})
    metadata = _strict_mapping(metadata, source=f"{source}:metadata")
    modes = _string_list(metadata.get("work-modes"), field="metadata.work-modes", allowed=VALID_MODES)
    tags = _string_list(metadata.get("work-tags"), field="metadata.work-tags")

    allow_implicit = True
    dependencies: list[dict[str, object]] = []
    openai_path = skill_root / "agents" / "openai.yaml"
    if openai_path.is_file():
        openai = _yaml_file(openai_path)
        policy = _strict_mapping(openai.get("policy", {}), source=f"{source}:policy")
        if "allow_implicit_invocation" in policy:
            if not isinstance(policy["allow_implicit_invocation"], bool):
                raise WorkError(
                    ExitCode.INPUT_FORMAT,
                    "invalid_skill_invocation_policy",
                    "allow_implicit_invocation must be a boolean.",
                    {"source": source},
                )
            allow_implicit = policy["allow_implicit_invocation"]
        dependencies = _normalize_dependencies(openai.get("dependencies"), source=str(openai_path))

    summary_fields = {
        "name": name.strip(),
        "description": description.strip(),
        "work_modes": modes,
        "work_tags": tags,
        "allow_implicit_invocation": allow_implicit,
        "dependencies": dependencies,
    }
    return {
        "id": _identity(name.strip(), scope, root_locator, source),
        "scope": scope,
        "root": root_locator,
        "source": source,
        **summary_fields,
        "summary_sha256": canonical_json_sha256(summary_fields),
    }


def _resolve_root(root: SkillRoot) -> Path:
    if root.scope not in VALID_SCOPES:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "invalid_skill_scope",
            "The skill root scope is invalid.",
            {"scope": root.scope},
        )
    try:
        resolved = root.path.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "skill_catalog_root_resolution_failed",
            "A skill catalog root could not be resolved.",
            {"scope": root.scope, "path": str(root.path)},
        ) from error
    if not resolved.is_dir():
        raise WorkError(
            ExitCode.IO_FAILURE,
            "skill_catalog_root_not_directory",
            "A skill catalog root is not a directory.",
            {"scope": root.scope, "path": str(resolved)},
        )
    return resolved


def _validate_root_locator(value: str) -> str:
    pure = PurePosixPath(value.replace("\\", "/"))
    if (
        not value
        or pure.is_absolute()
        or any(part in {"", ".", ".."} for part in pure.parts)
    ):
        raise WorkError(
            ExitCode.CLI_USAGE,
            "invalid_skill_root_locator",
            "A skill root locator must be a normalized scope-relative path.",
            {"locator": value},
        )
    return pure.as_posix()


def build_skill_catalog(
    roots: list[SkillRoot],
    *,
    disabled_sources: set[str] | None = None,
    excluded_names: set[str] | None = None,
) -> dict[str, object]:
    disabled = disabled_sources or set()
    excluded = excluded_names or {"work"}
    skills: list[dict[str, object]] = []
    unavailable: list[dict[str, object]] = []
    seen_paths: set[str] = set()
    for root in roots:
        root_locator = _validate_root_locator(root.locator)
        resolved_root = _resolve_root(root)
        for directory in sorted(resolved_root.iterdir(), key=lambda path: path.name.casefold()):
            skill_file = directory / "SKILL.md"
            if not skill_file.is_file():
                continue
            try:
                resolved_skill = directory.resolve(strict=True)
            except (OSError, RuntimeError) as error:
                unavailable.append(
                    {
                        "scope": root.scope,
                        "root": root_locator,
                        "source": f"{directory.name}/SKILL.md",
                        "code": "skill_path_resolution_failed",
                    }
                )
                continue
            path_identity = str(resolved_skill).casefold()
            if path_identity in seen_paths:
                continue
            seen_paths.add(path_identity)
            source = f"{directory.name}/SKILL.md"
            if source in disabled:
                continue
            try:
                summary = read_skill_summary(
                    resolved_skill,
                    scope=root.scope,
                    root_locator=root_locator,
                    source=source,
                )
                if summary["name"] in excluded:
                    continue
                skills.append(summary)
            except WorkError as error:
                unavailable.append(
                    {
                        "scope": root.scope,
                        "root": root_locator,
                        "source": source,
                        "code": error.code,
                        "message": error.message,
                    }
                )
    return {
        "schema": "work-skill-catalog/v1",
        "skills": skills,
        "unavailable": unavailable,
    }


def snapshot_catalog_skill(root: SkillRoot, source: str) -> dict[str, object]:
    root_locator = _validate_root_locator(root.locator)
    resolved_root = _resolve_root(root)
    pure_source = PurePosixPath(source.replace("\\", "/"))
    if pure_source.is_absolute() or len(pure_source.parts) != 2 or pure_source.parts[1] != "SKILL.md" or ".." in pure_source.parts:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "invalid_skill_source",
            "A skill source must use <folder>/SKILL.md relative to its catalog root.",
            {"source": source},
        )
    skill_root = resolved_root / pure_source.parts[0]
    summary = read_skill_summary(
        skill_root.resolve(strict=True),
        scope=root.scope,
        root_locator=root_locator,
        source=pure_source.as_posix(),
    )
    bundle = snapshot_skill_bundle(skill_root)
    return {
        "schema": "work-skill-snapshot/v1",
        "skill": summary,
        "bundle": bundle,
    }


def parse_skill_root(value: str) -> SkillRoot:
    identity, separator, raw_path = value.partition("=")
    scope, locator_separator, locator = identity.partition(":")
    if not separator or not raw_path or not locator_separator or not locator:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "invalid_skill_root_argument",
            "Skill roots must use scope:locator=path syntax.",
            {"value": value},
        )
    return SkillRoot(scope=scope, locator=_validate_root_locator(locator), path=Path(raw_path))
