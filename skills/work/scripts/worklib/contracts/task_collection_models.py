from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field

from ..models.common.base import WorkContract


class TaskNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class InstructionSourceModel(TaskNestedModel):
    kind: str
    logical_name: str
    canonical_sha256: str


class TaskInstructionSelectionModel(TaskNestedModel):
    selected_paths: list[str]
    resolved_paths: list[str]
    sources: list[InstructionSourceModel]
    references: list[str]
    instructions_sha256: str


class DocumentInstructionSelectionModel(TaskNestedModel):
    sources: list[InstructionSourceModel]
    references: list[str]
    instructions_sha256: str


class TraceabilityModel(TaskNestedModel):
    goal_ids: list[str]
    deliverable_ids: list[str]
    acceptance_ids: list[str]
    milestone_ids: list[str] | None = None


class ExecutionDefaultsModel(TaskNestedModel):
    working_directory: str
    os: str
    shell: str


class TaskInputModel(TaskNestedModel):
    id: str
    kind: str
    source: str
    precondition: str


class TaskDecisionModel(TaskNestedModel):
    id: str
    statement: str
    rationale: str


class TaskFileModel(TaskNestedModel):
    id: str
    action: Literal["create", "modify", "move"]
    path: str | None = None
    source: str | None = None
    destination: str | None = None


class TaskRiskModel(TaskNestedModel):
    id: str
    condition: str
    impact: str
    mitigation: str


class TaskStepModel(TaskNestedModel):
    id: str
    action: str
    references: list[str]


class TaskCommandModel(TaskNestedModel):
    id: str
    mode: Literal["argv", "shell"]
    argv: list[str] | None = None
    script: str | None = None
    execution: ExecutionDefaultsModel | None = None


class TaskOperationModel(TaskNestedModel):
    id: str
    kind: str
    action: str
    target: str
    validation_id: str
    command_id: str | None = None


class TaskValidationModel(TaskNestedModel):
    id: str
    kind: Literal["automated", "manual"]
    command_ids: list[str] | None = None
    pass_condition: str | None = None
    confirmer: str | None = None
    criteria: str | None = None
    acceptance_ids: list[str] | None = None


class TaskItemContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-item/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "id", "title", "skill_id", "instruction_selection",
        "traceability", "goal", "dependencies", "inputs", "decisions",
        "files", "risks", "steps", "commands", "operations", "validations",
    )
    schema_: Literal["work-task-item/v1"] = Field(alias="schema")
    id: str
    title: str
    skill_id: str | None
    instruction_selection: TaskInstructionSelectionModel
    traceability: TraceabilityModel
    goal: str
    dependencies: list[str] | None = None
    inputs: list[TaskInputModel] | None = None
    decisions: list[TaskDecisionModel] | None = None
    files: list[TaskFileModel] | None = None
    risks: list[TaskRiskModel] | None = None
    steps: list[TaskStepModel]
    commands: list[TaskCommandModel] | None = None
    operations: list[TaskOperationModel] | None = None
    validations: list[TaskValidationModel]

class TaskArtifactsModel(TaskNestedModel):
    plan: str
    task: str
    execution: str


class TaskSourcePlanModel(TaskNestedModel):
    canonical_sha256: str
    hierarchy_selection_sha256: str


class TaskReferenceModel(TaskNestedModel):
    id: str
    path: str
    canonical_sha256: str


class TaskReadinessModel(TaskNestedModel):
    status: Literal["passed"]
    spec_id: str


class SharedTaskDecisionModel(TaskNestedModel):
    id: str
    statement: str
    rationale: str
    task_ids: list[str]


class TaskChangeEditModel(TaskNestedModel):
    artifact: Literal["task_index", "task_item"]
    operation: Literal["add", "replace", "remove"]
    path: str
    task_id: str | None = None
    before: Any | None = None
    after: Any | None = None


class TaskChangeModel(TaskNestedModel):
    id: str
    spec_id: str
    date: str
    reason: str
    affected_ids: list[str]
    edits: list[TaskChangeEditModel]
    plan_change_ids: list[str] | None = None


class TaskIndexContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-index/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "spec_id", "status", "title", "summary",
        "artifacts", "source_plan", "instruction_selection", "execution_defaults",
        "tasks", "decisions", "changes", "readiness",
    )
    schema_: Literal["work-task-index/v1"] = Field(alias="schema")
    requirement_id: str
    spec_id: str
    status: Literal["confirmed"]
    title: str
    summary: str
    artifacts: TaskArtifactsModel
    source_plan: TaskSourcePlanModel
    instruction_selection: DocumentInstructionSelectionModel
    execution_defaults: ExecutionDefaultsModel | None = None
    tasks: list[TaskReferenceModel]
    decisions: list[SharedTaskDecisionModel] | None = None
    changes: list[TaskChangeModel] | None = None
    readiness: TaskReadinessModel


class TaskItemValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-item-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "dependencies", "task_item_sha256",
    )
    schema_: Literal["work-task-item-validation/v1"] = Field(alias="schema")
    task_id: str
    dependencies: list[str]
    task_item_sha256: str


class TaskIndexValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-index-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "spec_id", "task_ids", "task_paths",
        "task_item_sha256", "task_index_sha256",
    )
    schema_: Literal["work-task-index-validation/v1"] = Field(alias="schema")
    requirement_id: str
    spec_id: str
    task_ids: list[str]
    task_paths: dict[str, str]
    task_item_sha256: dict[str, str]
    task_index_sha256: str


class TaskCollectionFingerprintItemModel(TaskNestedModel):
    id: str
    task_item_sha256: str


class TaskCollectionFingerprintContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-collection-fingerprint/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "task_index_sha256", "items")
    schema_: Literal["work-task-collection-fingerprint/v1"] = Field(alias="schema")
    task_index_sha256: str
    items: list[TaskCollectionFingerprintItemModel]


class TaskCollectionProjectionContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-collection-projection/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "spec_id", "status", "title", "summary",
        "artifacts", "source_plan", "instruction_selection", "execution_defaults",
        "decisions", "tasks", "changes", "readiness",
    )
    schema_: Literal["work-task-collection-projection/v1"] = Field(alias="schema")
    requirement_id: str
    spec_id: str
    status: Literal["confirmed"]
    title: str
    summary: str
    artifacts: dict[str, str]
    source_plan: dict[str, Any]
    instruction_selection: dict[str, Any]
    execution_defaults: dict[str, Any] | None = None
    decisions: list[dict[str, Any]] | None = None
    tasks: list[dict[str, Any]]
    changes: list[dict[str, Any]] | None = None
    readiness: dict[str, Any]


class TaskCollectionValidationContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-collection-validation/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "spec_id", "task_ids", "task_count",
        "task_index_sha256", "task_item_sha256", "task_collection_sha256",
        "source_plan_sha256", "instructions_sha256", "task_instructions_sha256",
        "task_skill_ids", "hierarchy_selection_sha256", "skill_selection_sha256",
        "collection_contract",
    )
    schema_: Literal["work-task-collection-validation/v1"] = Field(alias="schema")
    requirement_id: str
    spec_id: str
    task_ids: list[str]
    task_count: int
    task_index_sha256: str
    task_item_sha256: dict[str, str]
    task_collection_sha256: str
    source_plan_sha256: str
    instructions_sha256: str
    task_instructions_sha256: dict[str, str]
    task_skill_ids: dict[str, str | None]
    hierarchy_selection_sha256: str
    skill_selection_sha256: str
    collection_contract: dict[str, Any]


class DiagnosticCheckModel(TaskNestedModel):
    name: str
    status: Literal["passed", "failed", "not_checked"]
    requires: list[str] | None = None
    location: str | None = None


class DiagnosticIssueModel(TaskNestedModel):
    stage: str
    code: str
    location: str | None = None
    category: str
    message: str
    suggestion: str
    details: dict[str, Any]


class TaskCollectionDiagnosticsContract(WorkContract):
    contract_id: ClassVar[str] = "work-task-collection-diagnostics/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "status", "task_path", "raw_sha256", "format_status",
        "contract_status", "normal_use_allowed", "execution_binding_status",
        "repair_mode", "checks", "issues",
    )
    schema_: Literal["work-task-collection-diagnostics/v1"] = Field(alias="schema")
    status: Literal["valid", "blocked"]
    task_path: str
    raw_sha256: str | None
    format_status: Literal["passed", "failed", "not_checked"]
    contract_status: Literal["passed", "failed", "not_checked"]
    normal_use_allowed: bool
    execution_binding_status: Literal["passed", "failed", "not_checked"]
    repair_mode: Literal["review_required"]
    checks: list[DiagnosticCheckModel]
    issues: list[DiagnosticIssueModel]


_SELECTION_EXAMPLE = {
    "sources": [{"kind": "instruction", "logical_name": "task.general", "canonical_sha256": "a" * 64}],
    "references": [], "instructions_sha256": "b" * 64,
}
TaskItemContract.contract_example = {
    "schema": "work-task-item/v1", "id": "TASK-001", "title": "Example",
    "skill_id": None,
    "instruction_selection": {"selected_paths": [], "resolved_paths": ["general"], **_SELECTION_EXAMPLE},
    "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
    "goal": "Produce the result.",
    "steps": [{"id": "STEP-001", "action": "Validate.", "references": ["VAL-001"]}],
    "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "user", "criteria": "Approved."}],
}
TaskIndexContract.contract_example = {
    "schema": "work-task-index/v1", "requirement_id": "example",
    "spec_id": "TASK-SPEC-001", "status": "confirmed", "title": "Example",
    "summary": "Example tasks.",
    "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
    "source_plan": {"canonical_sha256": "c" * 64, "hierarchy_selection_sha256": "d" * 64},
    "instruction_selection": _SELECTION_EXAMPLE,
    "tasks": [{"id": "TASK-001", "path": "tasks/TASK-001.json", "canonical_sha256": "e" * 64}],
    "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
}
TaskItemValidationContract.contract_example = {
    "schema": "work-task-item-validation/v1", "task_id": "TASK-001",
    "dependencies": [], "task_item_sha256": "a" * 64,
}
TaskIndexValidationContract.contract_example = {
    "schema": "work-task-index-validation/v1", "requirement_id": "example",
    "spec_id": "TASK-SPEC-001", "task_ids": ["TASK-001"],
    "task_paths": {"TASK-001": "tasks/TASK-001.json"},
    "task_item_sha256": {"TASK-001": "a" * 64}, "task_index_sha256": "b" * 64,
}
TaskCollectionFingerprintContract.contract_example = {
    "schema": "work-task-collection-fingerprint/v1", "task_index_sha256": "a" * 64,
    "items": [{"id": "TASK-001", "task_item_sha256": "b" * 64}],
}
TaskCollectionProjectionContract.contract_example = {
    "schema": "work-task-collection-projection/v1", "requirement_id": "example",
    "spec_id": "TASK-SPEC-001", "status": "confirmed", "title": "Example",
    "summary": "Example tasks.",
    "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
    "source_plan": {"canonical_sha256": "a" * 64, "hierarchy_selection_sha256": "b" * 64},
    "instruction_selection": _SELECTION_EXAMPLE, "tasks": [],
    "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
}
TaskCollectionValidationContract.contract_example = {
    "schema": "work-task-collection-validation/v1", "requirement_id": "example",
    "spec_id": "TASK-SPEC-001", "task_ids": ["TASK-001"], "task_count": 1,
    "task_index_sha256": "a" * 64, "task_item_sha256": {"TASK-001": "b" * 64},
    "task_collection_sha256": "c" * 64, "source_plan_sha256": "d" * 64,
    "instructions_sha256": "e" * 64, "task_instructions_sha256": {"TASK-001": "f" * 64},
    "task_skill_ids": {"TASK-001": None}, "hierarchy_selection_sha256": "1" * 64,
    "skill_selection_sha256": "2" * 64, "collection_contract": {},
}
TaskCollectionDiagnosticsContract.contract_example = {
    "schema": "work-task-collection-diagnostics/v1", "status": "valid",
    "task_path": "outputs/work/tasks/example/index.json", "raw_sha256": "a" * 64,
    "format_status": "passed", "contract_status": "passed",
    "normal_use_allowed": True, "execution_binding_status": "not_checked",
    "repair_mode": "review_required", "checks": [], "issues": [],
}
