"""Execution inspection data models."""
from __future__ import annotations
from typing import Any, ClassVar, Literal
from pydantic import BaseModel, ConfigDict, Field
from ...protocol import SHA256_PATTERN
from ..common.base import WorkContract

class InspectionNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra='forbid', frozen=True, validate_default=True)


class InstructionSourceModel(InspectionNestedModel):
    kind: str
    logical_name: str
    canonical_sha256: str = Field(pattern=SHA256_PATTERN)
    compatibility_revision: int = Field(gt=0)


class ExecuteInstructionSelectionModel(InspectionNestedModel):
    selected_paths: list[str]
    resolved_paths: list[str]
    sources: list[InstructionSourceModel]
    references: list[str]
    instructions_sha256: str = Field(pattern=SHA256_PATTERN)


class ExecuteSkillSelectionModel(InspectionNestedModel):
    schema_: Literal['work-skill-selection/v1'] = Field(alias='schema')
    decision: Literal['external_skills', 'base_only']
    skills: list[dict[str, Any]]
    selection_sha256: str = Field(pattern=SHA256_PATTERN)


class PreflightInputModel(InspectionNestedModel):
    id: str
    kind: Literal['project_state', 'task_output', 'user_provided', 'external']
    status: Literal['ready']
    resolved_source: str | None = None


class PreflightFileModel(InspectionNestedModel):
    id: str
    action: Literal['create', 'modify', 'move']
    status: Literal['ready']
    path: str | None = None
    source: str | None = None
    destination: str | None = None


class ExecutePreflightContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execute-preflight/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'requirement_id', 'task_spec_id', 'task_id', 'skill_id', 'task_path', 'execution_dir', 'index_path', 'task_status', 'dependencies', 'confirmed_inputs', 'inputs', 'files', 'task_collection_sha256', 'task_index_sha256', 'task_item_sha256', 'hierarchy_selection_sha256', 'task_instructions_sha256', 'execute_instructions_sha256', 'execute_instruction_selection', 'execute_skill_selection', 'index_sha256', 'eligibility')
    schema_: Literal['work-execute-preflight/v1'] = Field(alias='schema')
    requirement_id: str
    task_spec_id: str
    task_id: str
    skill_id: str | None
    task_path: str
    execution_dir: str
    index_path: str
    task_status: str
    dependencies: list[str]
    confirmed_inputs: list[str]
    inputs: list[PreflightInputModel]
    files: list[PreflightFileModel]
    task_collection_sha256: str = Field(pattern=SHA256_PATTERN)
    task_index_sha256: str = Field(pattern=SHA256_PATTERN)
    task_item_sha256: str = Field(pattern=SHA256_PATTERN)
    hierarchy_selection_sha256: str = Field(pattern=SHA256_PATTERN)
    task_instructions_sha256: str = Field(pattern=SHA256_PATTERN)
    execute_instructions_sha256: str = Field(pattern=SHA256_PATTERN)
    execute_instruction_selection: ExecuteInstructionSelectionModel
    execute_skill_selection: ExecuteSkillSelectionModel
    index_sha256: str = Field(pattern=SHA256_PATTERN)
    eligibility: Literal['passed']


class WorktreeRecordModel(InspectionNestedModel):
    index_status: str
    worktree_status: str
    path: str
    original_path: str | None = None


class ExecuteWorktreeSnapshotContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execute-worktree-snapshot/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'records')
    schema_: Literal['work-execute-worktree-snapshot/v1'] = Field(alias='schema')
    records: list[WorktreeRecordModel]


class WorktreeChangeModel(WorktreeRecordModel):
    path_classification: Literal['target_task', 'completed_dependency', 'unrelated']
    matched_task_ids: list[str] | None = None


class WorktreeCountsModel(InspectionNestedModel):
    staged: int = Field(ge=0)
    unstaged: int = Field(ge=0)
    untracked: int = Field(ge=0)
    target_task: int = Field(ge=0)
    completed_dependency: int = Field(ge=0)
    unrelated: int = Field(ge=0)


class ExecuteWorktreeContract(WorkContract):
    contract_id: ClassVar[str] = 'work-execute-worktree/v1'
    contract_kind: ClassVar[Literal['response']] = 'response'
    canonical_order: ClassVar[tuple[str, ...]] = ('schema', 'requirement_id', 'task_spec_id', 'task_id', 'task_collection_sha256', 'task_index_sha256', 'task_item_sha256', 'task_instructions_sha256', 'execute_instructions_sha256', 'task_status', 'task_path', 'index_sha256', 'execution_dir', 'snapshot_sha256', 'review_status', 'excluded_execution_change_count', 'counts', 'changes')
    schema_: Literal['work-execute-worktree/v1'] = Field(alias='schema')
    requirement_id: str
    task_spec_id: str
    task_id: str
    task_collection_sha256: str = Field(pattern=SHA256_PATTERN)
    task_index_sha256: str = Field(pattern=SHA256_PATTERN)
    task_item_sha256: str = Field(pattern=SHA256_PATTERN)
    task_instructions_sha256: str = Field(pattern=SHA256_PATTERN)
    execute_instructions_sha256: str = Field(pattern=SHA256_PATTERN)
    task_status: str
    task_path: str
    index_sha256: str = Field(pattern=SHA256_PATTERN)
    execution_dir: str
    snapshot_sha256: str = Field(pattern=SHA256_PATTERN)
    review_status: Literal['required', 'clean']
    excluded_execution_change_count: int = Field(ge=0)
    counts: WorktreeCountsModel
    changes: list[WorktreeChangeModel]

ExecutePreflightContract.contract_example = {
    "schema": "work-execute-preflight/v1", "requirement_id": "example",
    "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001", "skill_id": None,
    "task_path": "outputs/work/tasks/example/index.json",
    "execution_dir": "outputs/work/executions/example",
    "index_path": "outputs/work/executions/example/index.json", "task_status": "pending",
    "dependencies": [], "confirmed_inputs": [], "inputs": [], "files": [],
    "task_collection_sha256": "0" * 64, "task_index_sha256": "0" * 64,
    "task_item_sha256": "0" * 64, "hierarchy_selection_sha256": "0" * 64,
    "task_instructions_sha256": "0" * 64, "execute_instructions_sha256": "0" * 64,
    "execute_instruction_selection": {"selected_paths": [], "resolved_paths": [],
        "sources": [], "references": [], "instructions_sha256": "0" * 64},
    "execute_skill_selection": {"schema": "work-skill-selection/v1",
        "decision": "base_only", "skills": [], "selection_sha256": "0" * 64},
    "index_sha256": "0" * 64, "eligibility": "passed",
}


ExecuteWorktreeSnapshotContract.contract_example = {
    "schema": "work-execute-worktree-snapshot/v1", "records": [],
}


ExecuteWorktreeContract.contract_example = {
    "schema": "work-execute-worktree/v1", "requirement_id": "example",
    "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001",
    "task_collection_sha256": "0" * 64, "task_index_sha256": "0" * 64,
    "task_item_sha256": "0" * 64, "task_instructions_sha256": "0" * 64,
    "execute_instructions_sha256": "0" * 64, "task_status": "pending",
    "task_path": "outputs/work/tasks/example/index.json", "index_sha256": "0" * 64,
    "execution_dir": "outputs/work/executions/example", "snapshot_sha256": "0" * 64,
    "review_status": "clean", "excluded_execution_change_count": 0,
    "counts": {"staged": 0, "unstaged": 0, "untracked": 0, "target_task": 0,
        "completed_dependency": 0, "unrelated": 0}, "changes": [],
}

__all__ = ["ExecuteInstructionSelectionModel", "ExecutePreflightContract", "ExecuteSkillSelectionModel", "ExecuteWorktreeContract", "ExecuteWorktreeSnapshotContract", "InspectionNestedModel", "InstructionSourceModel", "PreflightFileModel", "PreflightInputModel", "WorktreeChangeModel", "WorktreeCountsModel", "WorktreeRecordModel"]
