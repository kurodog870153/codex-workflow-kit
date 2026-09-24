"""Recovery data models."""
from __future__ import annotations
from pathlib import Path
from typing import Any, ClassVar, Literal, Self
from pydantic import Field, model_validator
from ..common.base import WorkContract
from ..common.errors import ExitCode, WorkError

RecoveryTransaction = Literal[
    "record_begin", "command_correction", "record_finish", "attempt_close",
    "correction", "deviation_record",
]

class ExecutionRecoveryRequestContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execution-recovery-request/v1'
    contract_kind: ClassVar[Literal['generated_request']] = 'generated_request'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'transaction', 'attempt_id', 'transaction_files')
    schema_: Literal['work-execution-recovery-request/v1'] = Field(alias='schema')
    transaction: RecoveryTransaction
    attempt_id: str
    transaction_files: list[str]

    @model_validator(mode='after')
    def validate_file_list(self) -> Self:
        if any((not item or Path(item).name != item or '/' in item or ('\\' in item) for item in self.transaction_files)):
            raise WorkError(ExitCode.CONTRACT, 'execution_recovery_invalid_file_name', 'Every transaction file must be a plain non-empty file name.')
        if self.transaction_files != sorted(self.transaction_files) or len(self.transaction_files) != len(set(self.transaction_files)):
            raise WorkError(ExitCode.CONTRACT, 'execution_recovery_noncanonical_file_list', 'transaction_files must be unique and sorted.')
        return self


class ExecutionRecoveryPrepareRequestContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execution-recovery-prepare-request/v1'
    contract_kind: ClassVar[Literal['semantic_request']] = 'semantic_request'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'transaction', 'attempt_id')
    schema_: Literal['work-execution-recovery-prepare-request/v1'] = Field(alias='schema')
    transaction: RecoveryTransaction
    attempt_id: str

    def to_recovery_request(self) -> ExecutionRecoveryRequestContract:
        return ExecutionRecoveryRequestContract.model_validate({'schema': 'work-execution-recovery-request/v1', 'transaction': self.transaction, 'attempt_id': self.attempt_id, 'transaction_files': []})


class RecoveryEvidenceContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execution-recovery-evidence/v1'
    contract_kind: ClassVar[Literal['artifact']] = 'artifact'
    canonical_order: ClassVar[tuple[str, ...]] = ('raw_sha256', 'size_bytes')
    raw_sha256: str
    size_bytes: int


class ExecutionRecoveryPrepareContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execution-recovery-prepare/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'status', 'request', 'task_id', 'attempt_path', 'index_path', 'lock', 'attempt_status', 'evidence', 'recovery_validation', 'recovery_authorized')
    schema_: Literal['work-execution-recovery-prepare/v1'] = Field(alias='schema')
    status: Literal['prepared']
    request: ExecutionRecoveryRequestContract
    task_id: str
    attempt_path: str
    index_path: str
    lock: dict[str, Any] | None
    attempt_status: str
    evidence: dict[str, RecoveryEvidenceContract]
    recovery_validation: Literal['requires_authorized_recover']
    recovery_authorized: Literal[False]


class ExecutionRecoveryContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execution-recovery/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'transaction', 'task_id', 'attempt_id', 'attempt_path', 'correction_id', 'correction_path', 'index_path', 'affected_task_ids', 'record_id', 'attempt_status', 'lock_status', 'status')
    schema_: Literal['work-execution-recovery/v1'] = Field(alias='schema')
    transaction: RecoveryTransaction
    task_id: str
    attempt_id: str
    attempt_path: str | None = None
    correction_id: str | None = None
    correction_path: str | None = None
    index_path: str
    affected_task_ids: list[str] | None = None
    record_id: str | None = None
    attempt_status: str | None = None
    lock_status: Literal['record_reserved', 'attempt_held', 'released']
    status: Literal['recovered']

ExecutionRecoveryRequestContract.contract_example = {
    "schema": "work-execution-recovery-request/v1",
    "transaction": "record_begin", "attempt_id": "ATTEMPT-001",
    "transaction_files": [".work-record-begin-TASK-001-ATTEMPT-001-VAL-001.tmp"],
}


ExecutionRecoveryPrepareRequestContract.contract_example = {
    "schema": "work-execution-recovery-prepare-request/v1",
    "transaction": "record_begin", "attempt_id": "ATTEMPT-001",
}


RecoveryEvidenceContract.contract_example = {
    "raw_sha256": "0" * 64, "size_bytes": 1,
}


ExecutionRecoveryPrepareContract.contract_example = {
    "schema": "work-execution-recovery-prepare/v1", "status": "prepared",
    "request": ExecutionRecoveryRequestContract.contract_example,
    "task_id": "TASK-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "lock": None, "attempt_status": "in_progress",
    "evidence": {"outputs/work/executions/example/index.json": RecoveryEvidenceContract.contract_example},
    "recovery_validation": "requires_authorized_recover",
    "recovery_authorized": False,
}


ExecutionRecoveryContract.contract_example = {
    "schema": "work-execution-recovery/v1", "transaction": "record_begin",
    "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "record_id": "VAL-001", "lock_status": "record_reserved",
    "status": "recovered",
}

__all__ = ["ExecutionRecoveryContract", "ExecutionRecoveryPrepareContract", "ExecutionRecoveryPrepareRequestContract", "ExecutionRecoveryRequestContract", "RecoveryEvidenceContract", "RecoveryTransaction"]
