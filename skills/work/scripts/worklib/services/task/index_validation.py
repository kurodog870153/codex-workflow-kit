from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from pydantic import ValidationError

from ...models.common.errors import ExitCode, WorkError
from ...technical.infrastructure.text_codec import canonical_sha256
from ...technical.infrastructure.json_contract import (
    parse_json_contract,
    render_json_contract,
    require_canonical_json_contract,
)
from ...technical.infrastructure.work_paths import (
    portable_path_identity,
    resolve_project_relative_path,
    resolve_task_collection_item_path,
    validate_task_collection_index_path,
    validate_task_item_path_aliases,
)
from ...models.task_collection import TaskIndexContract, TaskIndexValidationContract
from ...models.common.validation import ContractValuePolicy


nonempty_string = ContractValuePolicy.nonempty_string
sha256 = ContractValuePolicy.sha256
strict_keys = ContractValuePolicy.strict_keys


TASK_ID_PATTERN = re.compile(r"^TASK-(\d{3})$")
SPEC_ID_PATTERN = re.compile(r"^TASK-SPEC-(\d{3})$")


def _string_array(value: object, *, location: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A non-empty string array is required.",
            {"location": location},
        )
    result = [nonempty_string(item, location=f"{location}[]") for item in value]
    if len(result) != len(set(result)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_array_value",
            "Array values must be unique.",
            {"location": location},
        )
    return result


def _execution(value: object, *, location: str) -> None:
    execution = strict_keys(
        value,
        location=location,
        required={"working_directory", "os", "shell"},
    )
    for field in ("working_directory", "os", "shell"):
        nonempty_string(execution[field], location=f"{location}.{field}")


def _validate_changes(value: object, *, spec_id: str, task_ids: set[str]) -> None:
    if not isinstance(value, list) or not value:
        raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "changes must be non-empty.")
    previous = 0
    previous_spec = 0
    current_spec = int(spec_id.rsplit("-", 1)[1])
    for index, raw_change in enumerate(value):
        latest = index == len(value) - 1
        change = strict_keys(
            raw_change,
            location=f"changes[{index}]",
            required={"id", "spec_id", "date", "reason", "affected_ids", "edits"},
            optional={"plan_change_ids"},
        )
        match = re.fullmatch(r"TASK-CHANGE-(\d{3})", str(change["id"]))
        if not match or int(match.group(1)) <= previous:
            raise WorkError(ExitCode.CONTRACT, "invalid_or_unsorted_id", "Invalid TASK change ID.")
        previous = int(match.group(1))
        change_spec = re.fullmatch(r"TASK-SPEC-(\d{3})", str(change["spec_id"]))
        if (
            not change_spec
            or int(change_spec.group(1)) <= previous_spec
            or int(change_spec.group(1)) > current_spec
        ):
            raise WorkError(ExitCode.CONTRACT, "change_spec_mismatch", "Change spec_id history must be strictly increasing through the current spec.")
        previous_spec = int(change_spec.group(1))
        nonempty_string(change["date"], location="change.date")
        nonempty_string(change["reason"], location="change.reason")
        affected = _string_array(change["affected_ids"], location="change.affected_ids")
        if latest and any(item.split("/", 1)[0] not in task_ids for item in affected):
            raise WorkError(ExitCode.CONTRACT, "invalid_reference", "Unknown affected ID.")
        if any(not re.fullmatch(r"TASK-\d{3}(?:/.*)?", item) for item in affected):
            raise WorkError(ExitCode.CONTRACT, "invalid_reference", "Invalid affected TASK ID.")
        if "plan_change_ids" in change:
            for item in _string_array(change["plan_change_ids"], location="change.plan_change_ids"):
                if not re.fullmatch(r"PLAN-CHANGE-\d{3}", item):
                    raise WorkError(ExitCode.CONTRACT, "invalid_reference", "Invalid Plan change ID.")
        edits = change["edits"]
        if not isinstance(edits, list) or not edits:
            raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "Change edits must be non-empty.")
        for edit_index, raw_edit in enumerate(edits):
            edit = strict_keys(
                raw_edit,
                location=f"changes[{index}].edits[{edit_index}]",
                required={"artifact", "operation", "path"},
                optional={"task_id", "before", "after"},
            )
            artifact = edit["artifact"]
            if artifact == "task_item":
                if latest and edit.get("operation") != "remove" and edit.get("task_id") not in task_ids:
                    raise WorkError(ExitCode.CONTRACT, "invalid_reference", "A TASK item edit requires a known task_id.")
                if not re.fullmatch(r"TASK-\d{3}", str(edit.get("task_id"))):
                    raise WorkError(ExitCode.CONTRACT, "invalid_reference", "A TASK item edit requires a valid task_id.")
            elif artifact == "task_index":
                if "task_id" in edit:
                    raise WorkError(ExitCode.CONTRACT, "invalid_change_edit", "A TASK index edit must omit task_id.")
            else:
                raise WorkError(ExitCode.CONTRACT, "invalid_change_edit", "Invalid TASK change artifact.")
            expected = {
                "add": {"artifact", "operation", "path", "after"},
                "replace": {"artifact", "operation", "path", "before", "after"},
                "remove": {"artifact", "operation", "path", "before"},
            }
            if artifact == "task_item":
                expected = {key: fields | {"task_id"} for key, fields in expected.items()}
            operation = edit["operation"]
            if operation not in expected or set(edit) != expected[operation]:
                raise WorkError(ExitCode.CONTRACT, "invalid_change_edit", "Invalid TASK change edit fields.")
            if not nonempty_string(edit["path"], location="change.edit.path").startswith("/"):
                raise WorkError(ExitCode.CONTRACT, "invalid_json_pointer", "Change path must be a JSON Pointer.")
    if previous_spec != current_spec:
        raise WorkError(ExitCode.CONTRACT, "change_spec_mismatch", "The latest change must belong to the current spec.")


def render_task_index_contract(contract: dict[str, Any], *, ordered_contract: dict[str, Any]) -> bytes:
    try:
        checked = TaskIndexContract.model_validate(contract).to_canonical_dict()
    except ValidationError as error:
        raise TaskIndexContract._work_error(error) from error
    return render_json_contract(ordered_contract)


def validate_task_index_contract(
    raw: bytes,
    *,
    source: str,
    actual_index_path: str,
    project_root: Path,
    ordered_contract: dict[str, Any],
    required_fields: set[str],
    optional_fields: set[str],
    model: TaskIndexContract | None = None,
) -> dict[str, object]:
    if model is None:
        model = TaskIndexContract.parse_json_bytes(raw, source=source)
    contract = model.to_canonical_dict()
    strict_keys(contract, location="task_index", required=required_fields, optional=optional_fields)
    requirement_id = nonempty_string(contract["requirement_id"], location="requirement_id")
    spec_id = nonempty_string(contract["spec_id"], location="spec_id")
    spec_match = SPEC_ID_PATTERN.fullmatch(spec_id)
    if not spec_match:
        raise WorkError(ExitCode.CONTRACT, "invalid_task_spec_id", "Invalid TASK spec ID.")
    for field in ("title", "summary"):
        nonempty_string(contract[field], location=field)

    artifacts = strict_keys(
        contract["artifacts"],
        location="artifacts",
        required={"plan", "task", "execution"},
    )
    normalized_index, _ = validate_task_collection_index_path(
        project_root, requirement_id, artifacts["task"]
    )
    actual_normalized, _ = validate_task_collection_index_path(
        project_root, requirement_id, actual_index_path
    )
    if normalized_index != actual_normalized:
        raise WorkError(ExitCode.CONTRACT, "task_artifact_path_mismatch", "The TASK index path does not match artifacts.task.")
    plan, plan_path = resolve_project_relative_path(project_root, artifacts["plan"], field="artifacts.plan")
    execution, execution_path = resolve_project_relative_path(project_root, artifacts["execution"], field="artifacts.execution")
    if Path(plan).suffix != ".json" or Path(plan).stem != requirement_id:
        raise WorkError(ExitCode.CONTRACT, "plan_path_requirement_mismatch", "The Plan path must end with the requirement ID and .json.")
    if Path(execution).name != requirement_id:
        raise WorkError(ExitCode.CONTRACT, "execution_path_requirement_mismatch", "The execution path must end with the requirement ID.")
    identities = [portable_path_identity(path) for path in (plan_path, execution_path)]
    _, index_path = validate_task_collection_index_path(project_root, requirement_id, normalized_index)
    identities.append(portable_path_identity(index_path))
    if len(identities) != len(set(identities)):
        raise WorkError(ExitCode.CONTRACT, "artifact_path_alias", "Artifact paths must have distinct portable identities.")

    source_plan = strict_keys(
        contract["source_plan"],
        location="source_plan",
        required={"canonical_sha256", "hierarchy_selection_sha256"},
    )
    sha256(source_plan["canonical_sha256"], location="source_plan.canonical_sha256")
    sha256(source_plan["hierarchy_selection_sha256"], location="source_plan.hierarchy_selection_sha256")
    if "execution_defaults" in contract:
        _execution(contract["execution_defaults"], location="execution_defaults")

    raw_tasks = contract["tasks"]
    if not isinstance(raw_tasks, list) or not raw_tasks:
        raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "tasks must be non-empty.")
    task_ids: list[str] = []
    task_paths: dict[str, Path] = {}
    previous = 0
    for index, raw_reference in enumerate(raw_tasks):
        reference = strict_keys(
            raw_reference,
            location=f"tasks[{index}]",
            required={"id", "path", "canonical_sha256"},
        )
        task_id = nonempty_string(reference["id"], location=f"tasks[{index}].id")
        match = TASK_ID_PATTERN.fullmatch(task_id)
        if not match or int(match.group(1)) <= previous:
            raise WorkError(ExitCode.CONTRACT, "invalid_or_unsorted_id", "Invalid or unsorted TASK ID.")
        previous = int(match.group(1))
        _, task_paths[task_id] = resolve_task_collection_item_path(
            project_root, requirement_id, normalized_index, task_id, reference["path"]
        )
        sha256(reference["canonical_sha256"], location=f"tasks[{index}].canonical_sha256")
        task_ids.append(task_id)
    validate_task_item_path_aliases(task_paths)

    if "decisions" in contract:
        decisions = contract["decisions"]
        if not isinstance(decisions, list) or not decisions:
            raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "decisions must be non-empty.")
        for index, raw_decision in enumerate(decisions):
            decision = strict_keys(
                raw_decision,
                location=f"decisions[{index}]",
                required={"id", "statement", "rationale", "task_ids"},
            )
            for field in ("id", "statement", "rationale"):
                nonempty_string(decision[field], location=f"decisions[{index}].{field}")
            applies = _string_array(decision["task_ids"], location=f"decisions[{index}].task_ids")
            if len(applies) < 2 or any(item not in task_ids for item in applies):
                raise WorkError(ExitCode.CONTRACT, "invalid_shared_decision_scope", "A shared decision must apply to at least two TASKs.")

    if int(spec_match.group(1)) == 1 and "changes" in contract:
        raise WorkError(ExitCode.CONTRACT, "initial_task_has_changes", "Initial TASK index must omit changes.")
    if int(spec_match.group(1)) > 1 and "changes" not in contract:
        raise WorkError(ExitCode.CONTRACT, "task_changes_required", "A revised TASK index must contain changes.")
    if "changes" in contract:
        _validate_changes(contract["changes"], spec_id=spec_id, task_ids=set(task_ids))
    readiness = strict_keys(contract["readiness"], location="readiness", required={"status", "spec_id"})
    if readiness["status"] != "passed" or readiness["spec_id"] != spec_id:
        raise WorkError(ExitCode.CONTRACT, "invalid_readiness", "Formal TASK readiness must pass for the current spec.")

    require_canonical_json_contract(raw, contract=ordered_contract, source=source)
    return TaskIndexValidationContract.model_validate({
        "schema": "work-task-index-validation/v1",
        "requirement_id": requirement_id,
        "spec_id": spec_id,
        "task_ids": task_ids,
        "task_paths": {task_id: raw_tasks[index]["path"] for index, task_id in enumerate(task_ids)},
        "task_item_sha256": {task_id: raw_tasks[index]["canonical_sha256"] for index, task_id in enumerate(task_ids)},
        "task_index_sha256": canonical_sha256(raw, source=source),
    }).to_canonical_dict()
