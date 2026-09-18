"""Public contracts for AI-produced cross-file migration previews."""
from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .base import WorkContract


class MigrationNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class MigrationSourceEvidenceModel(MigrationNestedModel):
    path: str = Field(min_length=1, pattern=r"\S")
    raw_sha256: str = Field(pattern=r"^[0-9a-f]{64}$")


class MigrationCandidateDocumentModel(MigrationNestedModel):
    path: str = Field(min_length=1, pattern=r"\S")
    kind: Literal["plan", "task_index", "task_item", "execution_index"]
    task_id: str | None = None
    content: dict[str, Any]

    @model_validator(mode="after")
    def validate_identity(self) -> "MigrationCandidateDocumentModel":
        if (self.kind == "task_item") != (self.task_id is not None):
            raise ValueError("Only a task_item candidate requires task_id.")
        return self


class MigrationSemanticDecisionModel(MigrationNestedModel):
    id: str = Field(min_length=1, pattern=r"\S")
    question: str = Field(min_length=1, pattern=r"\S")
    resolution: str | None = None


class MigrationCheckModel(MigrationNestedModel):
    name: str
    status: Literal["passed", "failed"]
    code: str | None = None
    message: str | None = None


class MigrationDiffModel(MigrationNestedModel):
    path: str
    operation: Literal["add", "replace", "remove"]
    unified_diff: str


class SpecificationMigrationPreviewRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-migration-preview-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "sources", "candidates", "semantic_decisions",
    )
    field_constraints: ClassVar[dict[str, dict[str, Any]]] = {
        "sources": {"min_length": 1}, "candidates": {"min_length": 1},
    }

    schema_: Literal["work-spec-migration-preview-request/v1"] = Field(alias="schema")
    sources: list[MigrationSourceEvidenceModel] = Field(min_length=1)
    candidates: list[MigrationCandidateDocumentModel] = Field(min_length=1)
    semantic_decisions: list[MigrationSemanticDecisionModel]


class SpecificationMigrationPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-migration-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "documents", "diffs", "validator_results",
        "relationship_results", "unresolved_items", "fingerprint", "writable_ready",
    )

    schema_: Literal["work-spec-migration-preview/v1"] = Field(alias="schema")
    status: Literal["ready", "blocked"]
    documents: list[str]
    diffs: list[MigrationDiffModel]
    validator_results: list[MigrationCheckModel]
    relationship_results: list[MigrationCheckModel]
    unresolved_items: list[str]
    fingerprint: str = Field(pattern=r"^[0-9a-f]{64}$")
    writable_ready: bool

    @model_validator(mode="after")
    def validate_readiness(self) -> "SpecificationMigrationPreviewContract":
        passed = not self.unresolved_items and all(
            item.status == "passed"
            for item in self.validator_results + self.relationship_results
        )
        if self.writable_ready != passed or (self.status == "ready") != passed:
            raise ValueError("Migration readiness must match every check and semantic decision.")
        return self


class SpecificationMigrationPublicationContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-migration-publication/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "fingerprint", "transaction_approval_sha256",
        "journal", "completion_marker", "documents", "publication_status",
        "validator_results", "relationship_results",
    )

    schema_: Literal["work-spec-migration-publication/v1"] = Field(alias="schema")
    status: Literal["updated", "recovered"]
    fingerprint: str = Field(pattern=r"^[0-9a-f]{64}$")
    transaction_approval_sha256: str = Field(pattern=r"^[0-9a-f]{64}$")
    journal: str
    completion_marker: str
    documents: list[str]
    publication_status: Literal["published", "already_published"]
    validator_results: list[MigrationCheckModel]
    relationship_results: list[MigrationCheckModel]


SpecificationMigrationPreviewRequestContract.contract_example = {
    "schema": "work-spec-migration-preview-request/v1",
    "sources": [{"path": "outputs/work/plans/example.json", "raw_sha256": "0" * 64}],
    "candidates": [
        {"path": "outputs/work/plans/example.json", "kind": "plan", "content": {"schema": "work-plan/v1"}},
    ],
    "semantic_decisions": [],
}
SpecificationMigrationPreviewContract.contract_example = {
    "schema": "work-spec-migration-preview/v1", "status": "ready",
    "documents": ["outputs/work/plans/example.json"],
    "diffs": [{"path": "outputs/work/plans/example.json", "operation": "replace", "unified_diff": ""}],
    "validator_results": [{"name": "plan", "status": "passed"}],
    "relationship_results": [{"name": "candidate_set", "status": "passed"}],
    "unresolved_items": [], "fingerprint": "0" * 64, "writable_ready": True,
}
SpecificationMigrationPublicationContract.contract_example = {
    "schema": "work-spec-migration-publication/v1", "status": "updated",
    "fingerprint": "0" * 64, "transaction_approval_sha256": "1" * 64,
    "journal": "outputs/work/executions/example/.work-spec-migration-ABC.json",
    "completion_marker": "outputs/work/executions/example/.work-spec-migration-ABC.json.done",
    "documents": ["outputs/work/plans/example.json"], "publication_status": "published",
    "validator_results": [{"name": "plan", "status": "passed"}],
    "relationship_results": [{"name": "candidate_set", "status": "passed"}],
}
