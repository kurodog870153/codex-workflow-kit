"""Skill catalog and selection orchestration."""

from __future__ import annotations

from pathlib import Path

from ...models.skill import (
    SkillBundleContract,
    SkillCatalogContract,
    SkillRoot,
    SkillSelectionContract,
    SkillSelectionValidationContract,
    SkillSnapshotContract,
)
from ...services.skill_catalog import (
    build_skill_catalog,
    parse_skill_root,
    snapshot_catalog_skill,
    snapshot_skill_bundle,
)
from ...services.skill_selection import (
    build_skill_selection,
    parse_skill_selection_json,
    parse_skill_selection_request,
    validate_skill_roots,
    validate_skill_selection as validate_selection_value,
)


def _roots(values: list[SkillRoot] | list[str]) -> list[SkillRoot]:
    return [parse_skill_root(value) if isinstance(value, str) else value for value in values]


def _snapshots(value: object, roots: list[SkillRoot]) -> dict[tuple[str, str, str], dict[str, object]]:
    roots_by_identity = validate_skill_roots(roots)
    result: dict[tuple[str, str, str], dict[str, object]] = {}
    if not isinstance(value, dict) or not isinstance(value.get("skills"), list):
        return result
    for item in value["skills"]:
        if not isinstance(item, dict):
            continue
        identity = (item.get("scope"), item.get("root"))
        source = item.get("source")
        if not all(isinstance(part, str) and part.strip() for part in (*identity, source)):
            continue
        root = roots_by_identity.get(identity)
        if root is not None:
            key = (identity[0], identity[1], source)
            if key not in result:
                result[key] = snapshot_catalog_skill(root, source)
    return result


def catalog(roots: list[SkillRoot] | list[str], *, disabled_sources: set[str]) -> dict[str, object]:
    value = build_skill_catalog(_roots(roots), disabled_sources=disabled_sources)
    return SkillCatalogContract.model_validate(value).to_canonical_dict()


def snapshot(root: SkillRoot | str, source: str) -> dict[str, object]:
    parsed = parse_skill_root(root) if isinstance(root, str) else root
    value = snapshot_catalog_skill(parsed, source)
    return SkillSnapshotContract.model_validate(value).to_canonical_dict()


def bundle(root: Path) -> dict[str, object]:
    value = snapshot_skill_bundle(root)
    return SkillBundleContract.model_validate(value).to_canonical_dict()


def build_selection(raw: bytes, *, source: str, roots: list[SkillRoot] | list[str]) -> dict[str, object]:
    parsed_roots = _roots(roots)
    value = parse_skill_selection_request(raw, source=source)
    selected = build_skill_selection(value, roots=parsed_roots, snapshots=_snapshots(value, parsed_roots))
    validated = validate_selection_value(selected, roots=parsed_roots, snapshots=_snapshots(selected, parsed_roots))
    return SkillSelectionContract.model_validate(validated["skill_selection"]).to_canonical_dict()


def validate_selection(raw: bytes, *, roots: list[SkillRoot] | list[str]) -> dict[str, object]:
    parsed_roots = _roots(roots)
    parsed = parse_skill_selection_json(raw)
    value = validate_selection_value(parsed, roots=parsed_roots, snapshots=_snapshots(parsed, parsed_roots))
    return SkillSelectionValidationContract.model_validate(value).to_canonical_dict()


def validate_skill_selection(value: object, *, roots: list[SkillRoot]) -> dict[str, object]:
    return validate_selection_value(value, roots=roots, snapshots=_snapshots(value, roots))


__all__ = [
    "build_selection", "bundle", "catalog", "snapshot", "validate_selection", "validate_skill_selection",
]
