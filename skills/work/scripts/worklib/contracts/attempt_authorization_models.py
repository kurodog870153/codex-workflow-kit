from __future__ import annotations

import copy
from typing import ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .base import WorkContract
from .execution_deviation_models import ExecutionDeviationAction
from .task_collection_models import TaskCommandModel, TaskOperationModel, TaskValidationModel
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_json_sha256


REAPPROVAL_CONDITIONS = (
    "scope_expansion", "source_or_worktree_drift", "failure_divergence",
    "retry", "recovery", "unknown_result",
)


class AuthorizationNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True, validate_default=True)


class AttemptAuthorizationContract(WorkContract):
    contract_id: ClassVar[str] = "work-attempt-authorization/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "task_id", "commands", "validations", "modifiable_files",
        "working_directories", "external_operations", "allowed_deviations",
        "reapproval_conditions", "authorization_evidence",
    )

    schema_: Literal["work-attempt-authorization/v1"] = Field(alias="schema")
    task_id: str = Field(pattern=r"^TASK-[0-9]{3}$")
    commands: list[TaskCommandModel]
    validations: list[TaskValidationModel]
    modifiable_files: list[str]
    working_directories: list[str]
    external_operations: list[TaskOperationModel]
    allowed_deviations: list[ExecutionDeviationAction]
    reapproval_conditions: list[Literal[
        "scope_expansion", "source_or_worktree_drift", "failure_divergence",
        "retry", "recovery", "unknown_result",
    ]]
    authorization_evidence: str

    @model_validator(mode="after")
    def validate_closed_scope(self) -> Self:
        if tuple(self.reapproval_conditions) != REAPPROVAL_CONDITIONS:
            raise ValueError("Every fixed reapproval condition is required in canonical order.")
        if not self.authorization_evidence.strip():
            raise ValueError("authorization_evidence must be non-empty.")
        for values, label in (
            ([item.id for item in self.commands], "command"),
            ([item.id for item in self.validations], "validation"),
            ([item.id for item in self.external_operations], "external operation"),
            (self.modifiable_files, "modifiable file"),
            (self.working_directories, "working directory"),
        ):
            if len(values) != len(set(values)):
                raise ValueError(f"A {label} cannot be repeated.")
        return self


AttemptAuthorizationContract.contract_example = {
    "schema": "work-attempt-authorization/v1", "task_id": "TASK-001",
    "commands": [], "validations": [], "modifiable_files": [],
    "working_directories": [], "external_operations": [], "allowed_deviations": [],
    "reapproval_conditions": list(REAPPROVAL_CONDITIONS),
    "authorization_evidence": "User approved this exact Attempt scope.",
}


def minimal_authorization(task_id: str = "TASK-001") -> dict[str, object]:
    value = copy.deepcopy(AttemptAuthorizationContract.contract_example)
    value["task_id"] = task_id
    return AttemptAuthorizationContract.model_validate(value).to_canonical_dict()


def authorization_sha256(value: object) -> str:
    canonical = AttemptAuthorizationContract.model_validate(value).to_canonical_dict()
    return canonical_json_sha256(canonical)


def validate_authorization_scope(value: object, *, task: dict[str, object], defaults: object) -> dict[str, object]:
    manifest = AttemptAuthorizationContract.model_validate(value).to_canonical_dict()
    if manifest["task_id"] != task["id"]:
        raise WorkError(ExitCode.CONTRACT, "attempt_authorization_task_mismatch", "The authorization must target the selected TASK.")
    formal_commands = {item["id"]: item for item in task.get("commands", [])}
    formal_validations = {item["id"]: item for item in task.get("validations", [])}
    formal_external = {item["id"]: item for item in task.get("operations", []) if item["kind"] == "external_state"}
    for field, formal in (("commands", formal_commands), ("validations", formal_validations), ("external_operations", formal_external)):
        for item in manifest[field]:
            if formal.get(item["id"]) != item:
                raise WorkError(ExitCode.CONTRACT, "attempt_authorization_scope_expansion", "Authorization entries must exactly match the current TASK.", {"field": field, "id": item["id"]})
    formal_files = []
    for item in task.get("files", []):
        formal_files.extend(item[field] for field in ("path", "source", "destination") if field in item)
    if any(path not in formal_files for path in manifest["modifiable_files"]):
        raise WorkError(ExitCode.CONTRACT, "attempt_authorization_scope_expansion", "Modifiable files must be declared by the current TASK.")
    expected_directories = []
    for command in manifest["commands"]:
        execution = command.get("execution") or defaults
        directory = execution["working_directory"]
        if directory not in expected_directories:
            expected_directories.append(directory)
    if manifest["working_directories"] != expected_directories:
        raise WorkError(ExitCode.CONTRACT, "attempt_authorization_working_directories", "Working directories must exactly match the authorized commands.")
    return manifest
