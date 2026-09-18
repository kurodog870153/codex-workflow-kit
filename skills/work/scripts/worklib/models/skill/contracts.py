from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, ClassVar, Literal

from pydantic import Field

from ..common.base import WorkContract


@dataclass(frozen=True)
class SkillRoot:
    scope: str
    locator: str
    path: Path


class SkillCatalogContract(WorkContract):
    contract_id: ClassVar[str] = "work-skill-catalog/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "skills", "unavailable")
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-skill-catalog/v1", "skills": [], "unavailable": [],
    }
    schema_: Literal["work-skill-catalog/v1"] = Field(alias="schema")
    skills: list[dict[str, Any]]
    unavailable: list[dict[str, Any]]


class SkillBundleContract(WorkContract):
    contract_id: ClassVar[str] = "work-skill-bundle/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "files", "bundle_sha256")
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-skill-bundle/v1", "files": [], "bundle_sha256": "0" * 64,
    }
    schema_: Literal["work-skill-bundle/v1"] = Field(alias="schema")
    files: list[dict[str, str]]
    bundle_sha256: str


class SkillSnapshotContract(WorkContract):
    contract_id: ClassVar[str] = "work-skill-snapshot/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "skill", "bundle")
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-skill-snapshot/v1", "skill": {},
        "bundle": SkillBundleContract.contract_example,
    }
    schema_: Literal["work-skill-snapshot/v1"] = Field(alias="schema")
    skill: dict[str, Any]
    bundle: SkillBundleContract


class SkillSelectionContract(WorkContract):
    contract_id: ClassVar[str] = "work-skill-selection/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "decision", "skills", "selection_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-skill-selection/v1", "decision": "base_only",
        "skills": [], "selection_sha256": "0" * 64,
    }
    schema_: Literal["work-skill-selection/v1"] = Field(alias="schema")
    decision: Literal["external_skills", "base_only"]
    skills: list[dict[str, Any]]
    selection_sha256: str


class SkillSelectionValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-skill-selection-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "skill_selection",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-skill-selection-validation/v1", "status": "valid",
        "skill_selection": SkillSelectionContract.contract_example,
    }
    schema_: Literal["work-skill-selection-validation/v1"] = Field(alias="schema")
    status: Literal["valid"]
    skill_selection: SkillSelectionContract


