from __future__ import annotations

from ..contracts.skill import (
    SkillBundleContract,
    SkillCatalogContract,
    SkillSelectionContract,
    SkillSelectionValidationContract,
    SkillSnapshotContract,
)
from ..foundation.markdown import parse_json_contract
from .skill_catalog import (
    SkillRoot,
    build_skill_catalog,
    snapshot_catalog_skill,
)
from ..infrastructure.skill_fingerprint import snapshot_skill_bundle
from .skill_selection import build_skill_selection, validate_skill_selection_json


def catalog(roots: list[SkillRoot], *, disabled_sources: set[str]) -> dict[str, object]:
    value = build_skill_catalog(roots, disabled_sources=disabled_sources)
    return SkillCatalogContract.model_validate(value).to_canonical_dict()


def snapshot(root: SkillRoot, source: str) -> dict[str, object]:
    value = snapshot_catalog_skill(root, source)
    return SkillSnapshotContract.model_validate(value).to_canonical_dict()


def bundle(root) -> dict[str, object]:
    value = snapshot_skill_bundle(root)
    return SkillBundleContract.model_validate(value).to_canonical_dict()


def build_selection(raw: bytes, *, source: str, roots: list[SkillRoot]) -> dict[str, object]:
    value = build_skill_selection(parse_json_contract(raw, source=source), roots=roots)
    return SkillSelectionContract.model_validate(value).to_canonical_dict()


def validate_selection(raw: bytes, *, roots: list[SkillRoot]) -> dict[str, object]:
    value = validate_skill_selection_json(raw, roots=roots)
    return SkillSelectionValidationContract.model_validate(value).to_canonical_dict()
