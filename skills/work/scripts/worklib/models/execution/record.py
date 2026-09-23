"""Record data models."""
from __future__ import annotations
from typing import Any, ClassVar, Literal, Self
from pydantic import Field, model_validator
from ..common.base import WorkContract
from ..common.errors import ExitCode, WorkError

class RecordFinishRequestContract(WorkContract):
    contract_id: ClassVar[str] = 'work-record-finish-request/v1'
    contract_kind: ClassVar[Literal['semantic_request']] = 'semantic_request'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'record', 'modified_files', 'authorization_evidence')
    schema_: Literal['work-record-finish-request/v1'] = Field(alias='schema')
    record: dict[str, Any]
    modified_files: list[str] | None = None
    authorization_evidence: str | None = None

    @model_validator(mode='after')
    def validate_modified_files(self) -> Self:
        repeated = sorted(set(self.record) & {"id", "kind", "correction"})
        if repeated:
            raise WorkError(ExitCode.CONTRACT, 'record_finish_machine_fields', 'Record identity and command correction are derived from the active lock.', {'fields': repeated})
        if self.modified_files is not None:
            if not self.modified_files:
                raise WorkError(ExitCode.CONTRACT, 'record_finish_invalid_modified_files', 'modified_files must be a non-empty array when present.')
            if any((not item for item in self.modified_files)):
                raise WorkError(ExitCode.CONTRACT, 'record_finish_invalid_modified_file', 'Every modified file must be a non-empty string.')
        return self


class RecordBeginContract(WorkContract):
    contract_id: ClassVar[str] = 'work-record-begin/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'task_id', 'attempt_id', 'base_record_id', 'record_id', 'record_kind', 'index_path', 'lock_status')
    schema_: Literal['work-record-begin/v1'] = Field(alias='schema')
    task_id: str
    attempt_id: str
    base_record_id: str
    record_id: str
    record_kind: Literal['command', 'operation', 'validation']
    index_path: str
    lock_status: Literal['record_reserved']


class RecordFinishContract(WorkContract):
    contract_id: ClassVar[str] = 'work-record-finish/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'task_id', 'attempt_id', 'record_id', 'record_kind', 'attempt_path', 'index_path', 'record_status', 'lock_status')
    schema_: Literal['work-record-finish/v1'] = Field(alias='schema')
    task_id: str
    attempt_id: str
    record_id: str
    record_kind: Literal['command', 'operation', 'validation']
    attempt_path: str
    index_path: str
    record_status: Literal['recorded']
    lock_status: Literal['attempt_held']

RecordFinishRequestContract.contract_example = {
    "schema": "work-record-finish-request/v1",
    "record": {"outcome": "passed", "evidence": "The approved check passed."},
}


RecordBeginContract.contract_example = {
    "schema": "work-record-begin/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "base_record_id": "VAL-001",
    "record_id": "VAL-001",
    "record_kind": "validation",
    "index_path": "outputs/work/executions/example/index.json",
    "lock_status": "record_reserved",
}


RecordFinishContract.contract_example = {
    "schema": "work-record-finish/v1",
    "task_id": "TASK-001",
    "attempt_id": "ATTEMPT-001",
    "record_id": "VAL-001",
    "record_kind": "validation",
    "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    "index_path": "outputs/work/executions/example/index.json",
    "record_status": "recorded",
    "lock_status": "attempt_held",
}

__all__ = ["RecordBeginContract", "RecordFinishContract", "RecordFinishRequestContract"]
