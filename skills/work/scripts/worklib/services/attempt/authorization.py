from __future__ import annotations

import copy

from ...technical.foundation.fingerprint import canonical_json_sha256
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.authorization import AttemptAuthorizationContract


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
