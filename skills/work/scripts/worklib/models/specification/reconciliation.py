"""Contracts for reconciling recorded execution deviations into specifications."""
from __future__ import annotations

from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

from ...protocol import SHA256_PATTERN
from ..common.base import WorkContract
from .migration import (
    SpecificationMigrationPreviewRequestContract,
    SpecificationMigrationPreviewContract,
    SpecificationMigrationPublicationContract,
)


class SpecificationReconciliationPreviewRequestContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-reconciliation-preview-request/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "attempt_path", "choice", "deviation_ids", "migration",
    )
    field_references: ClassVar[dict[str, str]] = {
        "migration": "work-spec-migration-preview-request/v1",
    }

    schema_: Literal["work-spec-reconciliation-preview-request/v1"] = Field(alias="schema")
    attempt_path: str = Field(min_length=1, pattern=r"\S")
    choice: Literal["all", "selective", "retain_only"]
    deviation_ids: list[str]
    migration: SpecificationMigrationPreviewRequestContract | None = None

    @model_validator(mode="after")
    def validate_choice(self) -> "SpecificationReconciliationPreviewRequestContract":
        unique = sorted(set(self.deviation_ids))
        if unique != self.deviation_ids:
            raise ValueError("deviation_ids must be unique and sorted.")
        if self.choice == "selective" and (not self.deviation_ids or self.migration is None):
            raise ValueError("Selective reconciliation requires IDs and a migration candidate set.")
        if self.choice == "all" and (self.deviation_ids or self.migration is None):
            raise ValueError("All reconciliation derives IDs and requires a migration candidate set.")
        if self.choice == "retain_only" and (self.deviation_ids or self.migration is not None):
            raise ValueError("Retain-only reconciliation does not publish candidates.")
        return self


class ReconciliationLedgerEntryModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)

    deviation_id: str = Field(pattern=r"^DEVIATION-[0-9]{3}$")
    outcome: Literal["incorporated", "retained", "declined"]
    target: Literal["task_only", "plan_and_task", "retain_only"]
    attempt_sha256: str = Field(pattern=SHA256_PATTERN)
    reconciliation_fingerprint: str = Field(pattern=SHA256_PATTERN)


class SpecificationReconciliationLedgerContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-reconciliation-ledger/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "attempt_path", "entries")

    schema_: Literal["work-spec-reconciliation-ledger/v1"] = Field(alias="schema")
    attempt_path: str = Field(min_length=1, pattern=r"\S")
    entries: list[ReconciliationLedgerEntryModel]

    @model_validator(mode="after")
    def validate_entries(self) -> "SpecificationReconciliationLedgerContract":
        ids = [entry.deviation_id for entry in self.entries]
        if ids != sorted(set(ids)):
            raise ValueError("Ledger deviation IDs must be unique and sorted.")
        return self


class SpecificationReconciliationPreviewContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-reconciliation-preview/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "attempt_path", "attempt_sha256", "choice",
        "pending_deviation_ids", "selected_deviation_ids", "retained_deviation_ids",
        "deviation_classifications", "ledger_path", "ledger", "migration_preview", "fingerprint",
        "publication_required", "publication_ready",
    )
    field_references: ClassVar[dict[str, str]] = {
        "migration_preview": "work-spec-migration-preview/v1",
    }

    schema_: Literal["work-spec-reconciliation-preview/v1"] = Field(alias="schema")
    status: Literal["ready", "blocked"]
    attempt_path: str
    attempt_sha256: str = Field(pattern=SHA256_PATTERN)
    choice: Literal["all", "selective", "retain_only"]
    pending_deviation_ids: list[str]
    selected_deviation_ids: list[str]
    retained_deviation_ids: list[str]
    deviation_classifications: dict[
        str, Literal["task_only", "plan_and_task", "retain_only"]
    ]
    ledger_path: str
    ledger: SpecificationReconciliationLedgerContract
    migration_preview: SpecificationMigrationPreviewContract | None = None
    fingerprint: str = Field(pattern=SHA256_PATTERN)
    publication_required: bool
    publication_ready: bool

    @model_validator(mode="after")
    def validate_readiness(self) -> "SpecificationReconciliationPreviewContract":
        migration_required = self.choice != "retain_only"
        ready = not migration_required or (
            self.migration_preview is not None and self.migration_preview.writable_ready
        )
        if not self.publication_required or self.publication_ready != ready:
            raise ValueError("Reconciliation publication flags do not match the selected choice.")
        if (self.status == "ready") != ready:
            raise ValueError("Reconciliation status does not match migration readiness.")
        return self


class SpecificationReconciliationPublicationContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-reconciliation-publication/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "reconciliation_fingerprint", "attempt_path",
        "attempt_sha256", "selected_deviation_ids", "retained_deviation_ids",
        "ledger_path", "ledger_sha256", "publication", "deviation_classifications",
    )
    field_references: ClassVar[dict[str, str]] = {
        "publication": "work-spec-migration-publication/v1",
    }

    schema_: Literal["work-spec-reconciliation-publication/v1"] = Field(alias="schema")
    status: Literal["updated"]
    reconciliation_fingerprint: str = Field(pattern=SHA256_PATTERN)
    attempt_path: str
    attempt_sha256: str = Field(pattern=SHA256_PATTERN)
    selected_deviation_ids: list[str]
    retained_deviation_ids: list[str]
    ledger_path: str
    ledger_sha256: str = Field(pattern=SHA256_PATTERN)
    publication: SpecificationMigrationPublicationContract
    deviation_classifications: dict[
        str, Literal["task_only", "plan_and_task", "retain_only"]
    ]


_MIGRATION = SpecificationMigrationPreviewRequestContract.contract_example
SpecificationReconciliationPreviewRequestContract.contract_example = {
    "schema": "work-spec-reconciliation-preview-request/v1",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "choice": "all", "deviation_ids": [], "migration": _MIGRATION,
}
SpecificationReconciliationPreviewContract.contract_example = {
    "schema": "work-spec-reconciliation-preview/v1", "status": "ready",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "attempt_sha256": "0" * 64, "choice": "all",
    "pending_deviation_ids": ["DEVIATION-001"],
    "selected_deviation_ids": ["DEVIATION-001"], "retained_deviation_ids": [],
    "deviation_classifications": {"DEVIATION-001": "task_only"},
    "ledger_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/reconciliation.json",
    "ledger": {"schema": "work-spec-reconciliation-ledger/v1",
               "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
               "entries": [{"deviation_id": "DEVIATION-001", "outcome": "incorporated",
                            "target": "task_only", "attempt_sha256": "0" * 64,
                            "reconciliation_fingerprint": "1" * 64}]},
    "migration_preview": SpecificationMigrationPreviewContract.contract_example,
    "fingerprint": "1" * 64, "publication_required": True, "publication_ready": True,
}
SpecificationReconciliationPublicationContract.contract_example = {
    "schema": "work-spec-reconciliation-publication/v1", "status": "updated",
    "reconciliation_fingerprint": "1" * 64,
    "attempt_path": SpecificationReconciliationPreviewContract.contract_example["attempt_path"],
    "attempt_sha256": "0" * 64, "selected_deviation_ids": ["DEVIATION-001"],
    "retained_deviation_ids": [],
    "ledger_path": SpecificationReconciliationPreviewContract.contract_example["ledger_path"],
    "ledger_sha256": "2" * 64,
    "publication": SpecificationMigrationPublicationContract.contract_example,
    "deviation_classifications": {"DEVIATION-001": "task_only"},
}
SpecificationReconciliationLedgerContract.contract_example = SpecificationReconciliationPreviewContract.contract_example["ledger"]

__all__ = [
    "SpecificationReconciliationPreviewRequestContract",
    "ReconciliationLedgerEntryModel",
    "SpecificationReconciliationLedgerContract",
    "SpecificationReconciliationPreviewContract",
    "SpecificationReconciliationPublicationContract",
]
