"""Handoff use cases."""

from __future__ import annotations

import copy
from pathlib import Path

from ...models.common.errors import ExitCode, WorkError
from ...models.handoff import RETURN_FIELDS
from ...services.attempt import render_attempt_contract, validate_attempt_file, validate_execution_index
from ...services.handoff import (
    BASE_EXECUTE_REFERENCES, BLOCKING_STOPPED_TYPES, RECOVERY_REFERENCE,
    build_discussion_handoff, build_handoff, find_task_row, plan_item_ids, render_handoff_json_contract,
    require_index_identity, require_known_affected_ids, require_matching_handoff_source,
    task_affected_ids, task_fingerprints, validate_execute_instruction_selection,
    validate_execution_identity, validate_handoff_contract, validate_handoff_json_contract,
)
from ...services.handoff.source_io import (
    parse_json_contract, read_raw, raw_sha256,
    resolve_project_relative_path, resolve_task_collection_item_path,
    validate_execution_task_layout,
)
from ...services.specification.transaction import require_no_spec_update
from ...services.instruction.root import instruction_root
from ...services.skill_catalog import SkillRoot, parse_skill_root
from ...services.skill_selection import selection_sha256


def _strict_keys(value, *, location, required):
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "expected_object", "A JSON object is required.", {"location": location})
    missing, unknown = sorted(required - set(value)), sorted(set(value) - required)
    if missing or unknown:
        raise WorkError(ExitCode.CONTRACT, "invalid_object_fields", "The JSON object has missing or unknown fields.", {"location": location, "missing": missing, "unknown": unknown})
    return value


def _validate_execute_instructions(task, attempt, *, operation, operations):
    references = list(BASE_EXECUTE_REFERENCES)
    if "continued_from" in attempt:
        references.append(RECOVERY_REFERENCE)
    current = operations.build_instruction_selection(
        skill_root=instruction_root(), mode="execute",
        selected_paths=task["instruction_selection"]["selected_paths"],
        reference_names=references,
    )
    return validate_execute_instruction_selection(task, attempt, current, operation=operation)


def _read_validated_plan(project_root, plan_path, user_config_root, skill_roots, *, operations):
    """Keep the parsed Plan and its validation tied to the same source bytes."""
    normalized, resolved = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    raw = read_raw(resolved)
    validation = operations.validate_plan_contract(
        raw, source=str(resolved), actual_plan_path=normalized, project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
        _allow_task_index=True,
    )
    plan = parse_json_contract(raw, source=str(resolved))
    require_no_spec_update(project_root, plan["artifacts"]["execution"])
    return plan, validation, resolved, raw


def _build_handoff(project_root, direction, requirement_id, artifacts, source, payload):
    contract = build_handoff(direction, requirement_id, artifacts, source, payload)
    validate_handoff_contract(contract, project_root=project_root)
    return contract


def _require_unchanged_source(project_root, relative, resolved, raw, *, field):
    """Reject changed bytes or a redirected source path without changing state."""
    _, current = resolve_project_relative_path(project_root, relative, field=field)
    if current != resolved:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_changed", "A source artifact changed during handoff construction.", {"field": field})
    if isinstance(raw, dict):
        unchanged = all(
            read_raw(project_root / path) == snapshot
            for path, snapshot in raw.items()
        )
    else:
        unchanged = read_raw(current) == raw
    if not unchanged:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_changed", "A source artifact changed during handoff construction.", {"field": field})
    if field == "plan_path":
        plan = parse_json_contract(raw, source=str(resolved))
        require_no_spec_update(project_root, plan["artifacts"]["execution"])



def _read_validated_task(project_root, task_path, user_config_root, skill_roots, *, operations, validate_file_state=True):
    normalized, resolved = resolve_project_relative_path(project_root, task_path, field="task_path")
    raw = read_raw(resolved)
    validation = operations.load_task_collection(
        project_root,
        user_config_root,
        normalized,
        skill_roots=skill_roots,
        validate_file_state=validate_file_state,
        raw=raw,
    )
    index = parse_json_contract(raw, source=str(resolved))
    task = copy.deepcopy(validation["collection_contract"])
    task["artifacts"]["task"] = normalized
    task["source_plan"]["canonical_sha256"] = validation["source_plan_sha256"]
    snapshot: object = {normalized: raw}
    for reference in index["tasks"]:
        _, item_path = resolve_task_collection_item_path(
            project_root,
            index["requirement_id"],
            normalized,
            reference["id"],
            reference["path"],
        )
        snapshot[item_path] = read_raw(item_path)
    return task, validation, resolved, snapshot



def _task_skill_id(validation, task_id):
    if task_id not in validation["task_skill_ids"]:
        raise WorkError(ExitCode.CONTRACT, "handoff_task_not_found", "The explicitly selected TASK does not exist in the formal specification.", {"task_id": task_id})
    return validation["task_skill_ids"][task_id]


def _read_task_source_plan(project_root, task, validation, user_config_root, skill_roots, *, operations):
    plan, checked, resolved, raw = _read_validated_plan(
        project_root, task["artifacts"]["plan"], user_config_root, skill_roots,
        operations=operations,
    )
    if checked["plan_sha256"] != validation["source_plan_sha256"] or plan["artifacts"] != task["artifacts"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_changed", "The source Plan no longer matches the validated TASK.")
    return plan, resolved, raw



def _execute_return_request(request, direction):
    if direction not in {"execute_to_task", "execute_to_plan"}:
        raise WorkError(ExitCode.CONTRACT, "invalid_handoff_direction", "An Execute return direction is required.")
    payload = dict(_strict_keys(request, location="handoff_request", required={"summary", "reason", *RETURN_FIELDS}))
    reason = payload.pop("reason")
    return payload, reason


def _execute_skill_fingerprint(plan, skill_id):
    selected_skills = [skill for skill in plan["skill_selection"]["skills"] if skill["id"] == skill_id]
    return selection_sha256("base_only" if skill_id is None else "external_skills", selected_skills)


def _require_no_execution_transaction(execution_path):
    if any(execution_path.glob(".work-*.tmp")):
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_execution_recovery_required", "An execution transaction requires recovery.")


def _execute_return_contract(project_root, direction, task, validation, task_id, skill_id, fingerprint, context, payload, plan, *, attempt_sha256=None):
    contract = _build_handoff(project_root, direction, validation["requirement_id"], task["artifacts"], {
        "task_spec_id": validation["spec_id"], "task_id": task_id,
        **task_fingerprints(validation, task_id),
        "task_instructions_sha256": validation["task_instructions_sha256"][task_id],
        "skill_id": skill_id, "execute_skill_selection_sha256": fingerprint,
        "execution_context": context,
        **({"attempt_sha256": attempt_sha256} if attempt_sha256 is not None else {}),
    }, payload)
    require_known_affected_ids(contract, task_affected_ids(plan, task, task_id), code="handoff_unknown_affected_ids")
    return contract


def _require_unstarted_task(project_root, execution, index, row):
    if "lock" in index or row["status"] != "pending" or "latest_attempt" in row:
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_task_already_started", "Preflight return requires an unlocked, initial pending TASK without Attempt history.")
    task_directory = validate_execution_task_layout(project_root, f"{execution}/{row['id']}")
    if task_directory.exists() and (not task_directory.is_dir() or any(task_directory.iterdir())):
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_attempt_artifacts_present", "The target TASK execution directory must be absent or empty.")


def build_preflight_return_handoff(
    project_root: Path, request: object, *, direction: str, task_path: str,
    task_id: str, user_config_root: str, operations,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Describe a preflight specification defect, without claiming readiness."""
    payload, reason = _execute_return_request(request, direction)
    task, validation, resolved, raw = _read_validated_task(
        project_root, task_path, user_config_root, skill_roots,
        operations=operations, validate_file_state=False,
    )
    skill_id = _task_skill_id(validation, task_id)
    plan, plan_resolved, plan_raw = _read_task_source_plan(
        project_root, task, validation, user_config_root, skill_roots,
        operations=operations,
    )
    execution = task["artifacts"]["execution"]
    _, execution_path = resolve_project_relative_path(project_root, execution, field="execution_dir")
    _require_no_execution_transaction(execution_path)
    index_relative = f"{execution}/index.json"
    _, index_path = resolve_project_relative_path(project_root, index_relative, field="execution_index")
    index_raw = read_raw(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    index = parse_json_contract(index_raw, source=str(index_path))
    row = require_index_identity(index, task, validation)[task_id]
    _require_unstarted_task(project_root, execution, index, row)
    if row["skill_id"] != skill_id or index["skill_selection_sha256"] != validation["skill_selection_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_skill_identity_mismatch", "Execution skill bindings or fingerprints do not match the validated source.")
    contract = _execute_return_contract(
        project_root, direction, task, validation, task_id, skill_id, _execute_skill_fingerprint(plan, skill_id),
        {"attempt": {"status": "not_created"}, "phase": "preflight", "issue_type": "specification_defect", "reason": reason},
        payload, plan,
    )
    for relative, path, snapshot, field in (
        (task_path, resolved, raw, "task_path"),
        (task["artifacts"]["plan"], plan_resolved, plan_raw, "plan_path"),
        (index_relative, index_path, index_raw, "execution_index"),
    ):
        _require_unchanged_source(project_root, relative, path, snapshot, field=field)
    _require_no_execution_transaction(execution_path)
    _require_unstarted_task(project_root, execution, index, row)
    return contract


def build_plan_to_task_handoff(
    project_root: Path, request: object, *, plan_path: str, user_config_root: str,
    operations, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Derive all machine fields; the caller supplies summary and affected IDs.

    A valid result describes the current formal Plan, not authorization to
    create TASK artifacts, resume discussion, or execute work.
    """
    payload = _strict_keys(request, location="handoff_request", required={"summary", "affected_ids"})
    plan, validation, resolved, raw = _read_validated_plan(
        project_root, plan_path, user_config_root, skill_roots, operations=operations,
    )
    contract = _build_handoff(project_root, "plan_to_task", validation["requirement_id"], plan["artifacts"], {
        "plan_sha256": validation["plan_sha256"], "skill_selection_sha256": validation["skill_selection_sha256"],
    }, payload)
    require_known_affected_ids(contract, plan_item_ids(plan), code="handoff_unknown_plan_ids")
    _require_unchanged_source(project_root, plan_path, resolved, raw, field="plan_path")
    return contract



def verify_plan_to_task_handoff(
    project_root: Path, contract: dict[str, object], *, plan_path: str,
    user_config_root: str, operations, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Bind an incoming handoff to the explicitly selected current formal Plan."""
    result = validate_handoff_contract(contract, project_root=project_root)
    if contract["direction"] != "plan_to_task":
        raise WorkError(ExitCode.CONTRACT, "handoff_direction_mismatch", "This receiver requires a Plan-to-Task handoff.")
    expected = build_plan_to_task_handoff(
        project_root, {"summary": contract["summary"], "affected_ids": contract["affected_ids"]},
        plan_path=plan_path, user_config_root=user_config_root,
        operations=operations, skill_roots=skill_roots,
    )
    require_matching_handoff_source(contract, expected)
    return {
        **result, "schema": "work-handoff-source-validation/v1",
        "plan_path": expected["artifacts"]["plan"], "source": copy.deepcopy(expected["source"]),
    }


def build_task_to_execute_handoff(
    project_root: Path, request: object, *, task_path: str, task_id: str,
    user_config_root: str, operations, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Describe one explicitly selected formal TASK without performing preflight."""
    payload = _strict_keys(request, location="handoff_request", required={"summary"})
    task, validation, resolved, raw = _read_validated_task(
        project_root, task_path, user_config_root, skill_roots, operations=operations,
    )
    skill_id = _task_skill_id(validation, task_id)
    plan_path = task["artifacts"]["plan"]
    _, plan_resolved, plan_raw = _read_task_source_plan(
        project_root, task, validation, user_config_root, skill_roots,
        operations=operations,
    )
    contract = _build_handoff(project_root, "task_to_execute", validation["requirement_id"], task["artifacts"], {
        "task_spec_id": validation["spec_id"], "task_id": task_id,
        **task_fingerprints(validation, task_id),
        "task_instructions_sha256": validation["task_instructions_sha256"][task_id],
        "skill_id": skill_id,
        "skill_selection_sha256": validation["skill_selection_sha256"],
    }, payload)
    _require_unchanged_source(project_root, task_path, resolved, raw, field="task_path")
    _require_unchanged_source(project_root, plan_path, plan_resolved, plan_raw, field="plan_path")
    return contract


def verify_task_to_execute_handoff(
    project_root: Path, contract: dict[str, object], *, task_path: str, task_id: str,
    user_config_root: str, operations, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Bind an incoming handoff to one explicitly selected current formal TASK."""
    result = validate_handoff_contract(contract, project_root=project_root)
    if contract["direction"] != "task_to_execute":
        raise WorkError(ExitCode.CONTRACT, "handoff_direction_mismatch", "This receiver requires a Task-to-Execute handoff.")
    expected = build_task_to_execute_handoff(
        project_root, {"summary": contract["summary"]}, task_path=task_path, task_id=task_id,
        user_config_root=user_config_root, operations=operations, skill_roots=skill_roots,
    )
    require_matching_handoff_source(contract, expected)
    return {
        **result, "schema": "work-handoff-source-validation/v1",
        "task_path": expected["artifacts"]["task"], "source": copy.deepcopy(expected["source"]),
    }


def build_task_to_plan_handoff(
    project_root: Path, request: object, *, task_path: str, user_config_root: str,
    operations, task_id: str | None = None, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Return a formal specification to Plan; semantic decisions remain explicit."""
    payload = _strict_keys(request, location="handoff_request", required={"summary", *RETURN_FIELDS})
    task, validation, resolved, raw = _read_validated_task(
        project_root, task_path, user_config_root, skill_roots, operations=operations,
    )
    skill_id = _task_skill_id(validation, task_id) if task_id is not None else None
    source = {
        "plan_sha256": validation["source_plan_sha256"], "task_spec_id": validation["spec_id"],
        **task_fingerprints(validation, task_id),
        "skill_selection_sha256": validation["skill_selection_sha256"],
    }
    if task_id is not None:
        source.update(task_id=task_id, skill_id=skill_id)
    plan, plan_resolved, plan_raw = _read_task_source_plan(
        project_root, task, validation, user_config_root, skill_roots,
        operations=operations,
    )
    contract = _build_handoff(project_root, "task_to_plan", validation["requirement_id"], task["artifacts"], source, payload)
    require_known_affected_ids(contract, task_affected_ids(plan, task, task_id), code="handoff_unknown_affected_ids")
    _require_unchanged_source(project_root, task_path, resolved, raw, field="task_path")
    _require_unchanged_source(project_root, task["artifacts"]["plan"], plan_resolved, plan_raw, field="plan_path")
    return contract


def build_execute_return_handoff(
    project_root: Path, request: object, *, direction: str, task_path: str,
    task_id: str, attempt_id: str, user_config_root: str,
    operations, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Describe a specification defect using the latest closed Attempt."""
    payload, reason = _execute_return_request(request, direction)
    task, validation, resolved, raw = _read_validated_task(
        project_root, task_path, user_config_root, skill_roots,
        operations=operations, validate_file_state=False,
    )
    skill_id = _task_skill_id(validation, task_id)
    plan, plan_resolved, plan_raw = _read_task_source_plan(
        project_root, task, validation, user_config_root, skill_roots,
        operations=operations,
    )
    execution = task["artifacts"]["execution"]
    _, execution_path = resolve_project_relative_path(project_root, execution, field="execution_dir")
    _require_no_execution_transaction(execution_path)
    index_relative = f"{execution}/index.json"
    _, index_path = resolve_project_relative_path(project_root, index_relative, field="execution_index")
    index_raw = read_raw(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    index = parse_json_contract(index_raw, source=str(index_path))
    row = find_task_row(index, task_id)
    if "lock" in index or row.get("latest_attempt") != attempt_id:
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_attempt_not_current", "The selected Attempt must be latest and the execution index unlocked.")
    attempt_relative = f"{execution}/{task_id}/{attempt_id}/attempt.json"
    validate_attempt_file(project_root, attempt_relative)
    _, attempt_path = resolve_project_relative_path(project_root, attempt_relative, field="attempt_path")
    attempt_raw = read_raw(attempt_path)
    attempt = parse_json_contract(attempt_raw, source=str(attempt_path))
    if render_attempt_contract(attempt, project_root=project_root) != attempt_raw:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_changed", "The Attempt is no longer canonical.")
    if attempt["status"] not in {"stopped", "blocked"}:
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_attempt_not_closed", "A stopped or blocked Attempt is required.")
    validate_execution_identity(task_contract=task, task_validation=validation, index=index, attempt=attempt, task_id=task_id)
    expected_status = "blocked" if attempt["status"] == "blocked" or attempt["final_type"] in BLOCKING_STOPPED_TYPES else "pending_retry"
    if (attempt["attempt_id"] != attempt_id or row["status"] != expected_status
            or row.get("status_reason") != {"kind": "attempt", "ref": attempt_id}):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_attempt_state_mismatch", "The index does not describe the selected closed Attempt.")
    selected = next(entry for entry in task["tasks"] if entry["id"] == task_id)
    _validate_execute_instructions(
        selected, attempt, operation="handoff", operations=operations,
    )
    execute_fingerprint = _execute_skill_fingerprint(plan, skill_id)
    if (attempt["skill_id"] != skill_id or row["skill_id"] != skill_id
            or index["skill_selection_sha256"] != validation["skill_selection_sha256"]
            or attempt["execute_skill_selection_sha256"] != execute_fingerprint):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_skill_identity_mismatch", "Execution skill bindings or fingerprints do not match the validated source.")
    contract = _execute_return_contract(
        project_root, direction, task, validation, task_id, skill_id, execute_fingerprint,
        {"attempt": {"status": attempt["status"], "id": attempt_id},
         "phase": "execution", "issue_type": "specification_defect", "reason": reason}, payload, plan,
        attempt_sha256=raw_sha256(attempt_raw),
    )
    for relative, path, snapshot, field in (
        (task_path, resolved, raw, "task_path"),
        (task["artifacts"]["plan"], plan_resolved, plan_raw, "plan_path"),
        (index_relative, index_path, index_raw, "execution_index"),
        (attempt_relative, attempt_path, attempt_raw, "attempt_path"),
    ):
        _require_unchanged_source(project_root, relative, path, snapshot, field=field)
    _require_no_execution_transaction(execution_path)
    return contract


def verify_return_handoff(
    project_root: Path, contract: dict[str, object], *, direction: str,
    plan_path: str, task_path: str, user_config_root: str,
    task_id: str | None = None, attempt_id: str | None = None, preflight: bool = False,
    operations=None, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Verify saved return sources against independent receiver selections.

    Semantic proposals remain unapproved input. Missing historical fingerprints
    cannot be reconstructed as evidence of what the sender originally reviewed.
    """
    result = validate_handoff_contract(contract, project_root=project_root)
    if direction not in {"task_to_plan", "execute_to_plan", "execute_to_task"} or contract["direction"] != direction:
        raise WorkError(ExitCode.CONTRACT, "handoff_direction_mismatch", "The incoming return direction does not match this receiver.")
    normalized_plan, _ = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    normalized_task, _ = resolve_project_relative_path(project_root, task_path, field="task_path")
    if contract["artifacts"]["plan"] != normalized_plan or contract["artifacts"]["task"] != normalized_task:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_mismatch", "The return does not match the receiver's confirmed artifact paths.", {"fields": ["artifacts"]})
    payload = {key: contract[key] for key in {"summary", *RETURN_FIELDS}}
    options = dict(task_path=normalized_task, task_id=task_id, operations=operations,
                   user_config_root=user_config_root, skill_roots=skill_roots)
    if direction == "task_to_plan":
        if preflight or attempt_id is not None:
            raise WorkError(ExitCode.CONTRACT, "handoff_context_mismatch", "Task-to-Plan does not accept execution context arguments.")
        expected = build_task_to_plan_handoff(project_root, payload, **options)
    else:
        if task_id is None or (preflight == (attempt_id is not None)):
            raise WorkError(ExitCode.CONTRACT, "handoff_context_required", "Select one TASK and either preflight or a closed Attempt independently.")
        payload["reason"] = contract["source"]["execution_context"]["reason"]
        if preflight:
            expected = build_preflight_return_handoff(project_root, payload, direction=direction, **options)
        else:
            expected = build_execute_return_handoff(project_root, payload, direction=direction, attempt_id=attempt_id, **options)
    require_matching_handoff_source(contract, expected)
    return {**result, "schema": "work-handoff-source-validation/v1", "plan_path": normalized_plan,
            "task_path": normalized_task, "source": copy.deepcopy(expected["source"])}


def run_handoff(
    arguments: argparse.Namespace,
    project_root: Path,
    request: object | None,
    *,
    operations,
) -> dict[str, object]:
    if arguments.handoff_command == "build-discussion":
        return build_discussion_handoff(request.raw, source=request.source)
    if arguments.handoff_command in {"verify-task-to-plan", "verify-execute-to-plan", "verify-execute-to-task"}:
        return verify_return_handoff(
            project_root, parse_json_contract(request.raw, source=request.source),
            direction=arguments.handoff_command.removeprefix("verify-").replace("-", "_"),
            plan_path=arguments.plan_path, task_path=arguments.task_path, task_id=arguments.task_id,
            attempt_id=getattr(arguments, "attempt_id", None), preflight=getattr(arguments, "preflight", False),
            user_config_root=arguments.user_config_root,
            operations=operations,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.handoff_command == "verify-task-to-execute":
        return verify_task_to_execute_handoff(
            project_root, parse_json_contract(request.raw, source=request.source),
            task_path=arguments.task_path, task_id=arguments.task_id, user_config_root=arguments.user_config_root,
            operations=operations,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.handoff_command == "verify-plan-to-task":
        return verify_plan_to_task_handoff(
            project_root, parse_json_contract(request.raw, source=request.source),
            plan_path=arguments.plan_path, user_config_root=arguments.user_config_root,
            operations=operations,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.handoff_command in {"build-plan-to-task", "build-task-to-execute", "build-task-to-plan", "build-execute-to-task", "build-execute-to-plan"}:
        request = parse_json_contract(request.raw, source=request.source)
        common = {"user_config_root": arguments.user_config_root, "operations": operations,
                  "skill_roots": [parse_skill_root(root) for root in arguments.skill_root]}
        if arguments.handoff_command.startswith("build-execute-"):
            if arguments.preflight:
                return build_preflight_return_handoff(
                    project_root, request, direction=arguments.handoff_command.removeprefix("build-").replace("-", "_"),
                    task_path=arguments.task_path, task_id=arguments.task_id, **common,
                )
            return build_execute_return_handoff(
                project_root, request, direction=arguments.handoff_command.removeprefix("build-").replace("-", "_"),
                task_path=arguments.task_path, task_id=arguments.task_id, attempt_id=arguments.attempt_id, **common,
            )
        if arguments.handoff_command == "build-plan-to-task":
            return build_plan_to_task_handoff(project_root, request, plan_path=arguments.plan_path, **common)
        operation = build_task_to_plan_handoff if arguments.handoff_command == "build-task-to-plan" else build_task_to_execute_handoff
        return operation(project_root, request, task_path=arguments.task_path, task_id=arguments.task_id, **common)
    operation = (
        validate_handoff_json_contract
        if arguments.handoff_command == "validate"
        else render_handoff_json_contract
    )
    return operation(
        request.raw,
        source=request.source,
        project_root=project_root,
    )
