from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field, model_validator

from ..common.base import WorkContract


class SourceImpactContract(WorkContract):
    contract_id: ClassVar[str] = "work-source-impact/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "refreshable", "changed_sources", "affected_requirements", "affected_files",
        "blocked", "requirements",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-source-impact/v1", "status": "valid", "refreshable": True,
        "changed_sources": 0, "affected_requirements": 0,
        "affected_files": 0, "blocked": 0, "requirements": [],
    }

    schema_: Literal["work-source-impact/v1"] = Field(alias="schema")
    status: Literal["valid", "changes_detected", "review_required"]
    refreshable: bool
    changed_sources: int
    affected_requirements: int
    affected_files: int
    blocked: int
    requirements: list[dict[str, Any]]


class SourceRefreshPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-source-refresh-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "requirement_id", "changed_sources", "affected",
        "blocked", "files", "approved_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-source-refresh-preview/v1", "status": "refreshable",
        "requirement_id": "example", "changed_sources": 1,
        "affected": {"plans": 1, "task_items": 1, "task_indexes": 1, "execution_indexes": 1},
        "blocked": [], "files": [], "approved_sha256": "0" * 64,
    }

    schema_: Literal["work-source-refresh-preview/v1"] = Field(alias="schema")
    status: Literal["valid", "refreshable", "review_required"]
    requirement_id: str
    changed_sources: int
    affected: dict[str, int]
    blocked: list[dict[str, Any]]
    files: list[dict[str, Any]]
    approved_sha256: str


class SourceRefreshPublicationContract(WorkContract):
    contract_id: ClassVar[str] = "work-source-refresh-publication/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "requirement_id", "approved_sha256",
        "transaction_approval_sha256", "journal", "completion_marker", "updated_files",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-source-refresh-publication/v1", "status": "updated",
        "requirement_id": "example", "approved_sha256": "0" * 64,
        "transaction_approval_sha256": "1" * 64,
        "journal": "outputs/work/executions/example/.work-source-refresh-000.json",
        "completion_marker": "outputs/work/executions/example/.work-source-refresh-000.json.done",
        "updated_files": [],
    }

    schema_: Literal["work-source-refresh-publication/v1"] = Field(alias="schema")
    status: Literal["updated", "already_completed"]
    requirement_id: str
    approved_sha256: str
    transaction_approval_sha256: str
    journal: str
    completion_marker: str
    updated_files: list[str]


class SourceRefreshBatchPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-source-refresh-batch-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "requirements", "approved_sha256",
    )

    schema_: Literal["work-source-refresh-batch-preview/v1"] = Field(alias="schema")
    status: Literal["valid", "refreshable", "review_required"]
    requirements: list[SourceRefreshPreviewContract]
    approved_sha256: str


class SourceRefreshBatchPublicationContract(WorkContract):
    contract_id: ClassVar[str] = "work-source-refresh-batch-publication/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "semantics", "approved_sha256", "record_path",
        "requirements", "completed_requirement_ids", "publications",
    )

    schema_: Literal["work-source-refresh-batch-publication/v1"] = Field(alias="schema")
    status: Literal["in_progress", "updated", "already_completed"]
    semantics: Literal["recoverable_sequential"]
    approved_sha256: str
    record_path: str
    requirements: list[dict[str, str]]
    completed_requirement_ids: list[str]
    publications: list[SourceRefreshPublicationContract]

    @model_validator(mode="after")
    def validate_progress(self) -> "SourceRefreshBatchPublicationContract":
        ids = [row.get("requirement_id") for row in self.requirements]
        if ids != sorted(set(ids)):
            raise ValueError("Batch requirements must be unique and sorted.")
        if self.completed_requirement_ids != ids[:len(self.publications)]:
            raise ValueError("Batch completion must be an ordered Requirement prefix.")
        if [row.requirement_id for row in self.publications] != self.completed_requirement_ids:
            raise ValueError("Batch publications must match completed Requirements.")
        if self.status != "in_progress" and len(self.publications) != len(ids):
            raise ValueError("A completed batch must publish every Requirement.")
        return self


SourceRefreshBatchPreviewContract.contract_example = {
    "schema": "work-source-refresh-batch-preview/v1", "status": "refreshable",
    "requirements": [SourceRefreshPreviewContract.contract_example], "approved_sha256": "2" * 64,
}
SourceRefreshBatchPublicationContract.contract_example = {
    "schema": "work-source-refresh-batch-publication/v1", "status": "updated",
    "semantics": "recoverable_sequential", "approved_sha256": "2" * 64,
    "record_path": "outputs/work/transactions/pending/source-refresh-batch/2.json",
    "requirements": [{"requirement_id": "example", "approved_sha256": "0" * 64}],
    "completed_requirement_ids": ["example"],
    "publications": [SourceRefreshPublicationContract.contract_example],
}


class InstructionMigrationPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-instruction-migration-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "requirement_id", "router_compatibility_revision",
        "affected", "excluded", "files", "approved_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-instruction-migration-preview/v1",
        "status": "migration_required", "requirement_id": "example",
        "router_compatibility_revision": 3,
        "affected": {"plans": 1, "task_items": 1, "task_indexes": 1, "execution_indexes": 1},
        "excluded": [], "files": [], "approved_sha256": "0" * 64,
    }

    schema_: Literal["work-instruction-migration-preview/v1"] = Field(alias="schema")
    status: Literal["current", "migration_required", "review_required"]
    requirement_id: str
    router_compatibility_revision: int
    affected: dict[str, int]
    excluded: list[dict[str, Any]]
    files: list[dict[str, Any]]
    approved_sha256: str


class InstructionMigrationPublicationContract(WorkContract):
    contract_id: ClassVar[str] = "work-instruction-migration-publication/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "requirement_id", "approved_sha256",
        "transaction_approval_sha256", "journal", "completion_marker", "updated_files",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-instruction-migration-publication/v1", "status": "updated",
        "requirement_id": "example", "approved_sha256": "0" * 64,
        "transaction_approval_sha256": "1" * 64,
        "journal": "outputs/work/executions/example/.work-instruction-migration-000.json",
        "completion_marker": "outputs/work/executions/example/.work-instruction-migration-000.json.done",
        "updated_files": [],
    }

    schema_: Literal["work-instruction-migration-publication/v1"] = Field(alias="schema")
    status: Literal["updated", "already_completed"]
    requirement_id: str
    approved_sha256: str
    transaction_approval_sha256: str
    journal: str
    completion_marker: str
    updated_files: list[str]


__all__ = [
    "InstructionMigrationPreviewContract", "InstructionMigrationPublicationContract",
    "SourceImpactContract", "SourceRefreshPreviewContract", "SourceRefreshPublicationContract",
    "SourceRefreshBatchPreviewContract", "SourceRefreshBatchPublicationContract",
]
