"""Build and verify conversation handoffs against source artifacts without writing."""

from __future__ import annotations

import copy
from pathlib import Path

from ..contracts.handoff import DIRECTION_STAGES, HANDOFF_MARKER, RETURN_FIELDS, validate_handoff_contract
from ..contracts.plan import ID_PREFIXES, validate_plan_contract
from ..contracts.task import validate_task_contract
from ..contracts.attempt import render_attempt_contract, validate_attempt_file
from ..contracts.execution_index import validate_execution_index
from ..contracts.validation import strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.spec_update import require_no_spec_update
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path, validate_execution_task_layout
from ..skills.catalog import SkillRoot
from ..skills.selection import selection_sha256
from ..execution.context import find_task_row, validate_execution_identity
from ..execution.instructions import validate_execute_instructions
from ..execution.attempt_close import BLOCKING_STOPPED_TYPES
from ..execution.preflight import _require_index_identity


def _read_validated_plan(project_root, plan_path, user_config_root, skill_roots):
    """Keep the parsed Plan and its validation tied to the same source bytes."""
    normalized, resolved = resolve_project_relative_path(project_root, plan_path, field="plan_path")
    raw = read_raw(resolved)
    validation = validate_plan_contract(
        raw, source=str(resolved), actual_plan_path=normalized, project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
    )
    plan = parse_json_contract(raw, source=str(resolved))
    require_no_spec_update(project_root, plan["artifacts"]["execution"])
    return plan, validation, resolved, raw


def _build_handoff(project_root, direction, requirement_id, artifacts, source, payload):
    source_stage, target_stage = DIRECTION_STAGES[direction]
    contract = {
        "schema": "work-handoff/v1", "marker": HANDOFF_MARKER, "direction": direction,
        "requirement_id": requirement_id, "artifacts": copy.deepcopy(artifacts),
        "source": {"stage": source_stage, **copy.deepcopy(source)},
        "target": {"stage": target_stage}, **copy.deepcopy(payload),
    }
    validate_handoff_contract(contract, project_root=project_root)
    return contract


def _require_unchanged_source(project_root, relative, resolved, raw, *, field):
    """Reject changed bytes or a redirected source path without changing state."""
    _, current = resolve_project_relative_path(project_root, relative, field=field)
    if current != resolved or read_raw(current) != raw:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_changed", "A source artifact changed during handoff construction.", {"field": field})
    if field == "plan_path":
        plan = parse_json_contract(raw, source=str(resolved))
        require_no_spec_update(project_root, plan["artifacts"]["execution"])


def _plan_item_ids(plan):
    return {item["id"] for group in ID_PREFIXES for item in plan.get(group, [])}


def _require_known_affected_ids(contract, known_ids, *, code):
    unknown = [identifier for identifier in contract["affected_ids"] if identifier not in known_ids]
    if unknown:
        raise WorkError(ExitCode.CONTRACT, code, "Affected IDs must exist in the validated source scope.", {"affected_ids": unknown})


def _read_validated_task(project_root, task_path, user_config_root, skill_roots, *, validate_file_state=True):
    normalized, resolved = resolve_project_relative_path(project_root, task_path, field="task_path")
    raw = read_raw(resolved)
    validation = validate_task_contract(
        raw, source=str(resolved), actual_task_path=normalized, project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots, validate_file_state=validate_file_state,
    )
    return parse_json_contract(raw, source=str(resolved)), validation, resolved, raw


def _task_skill_id(validation, task_id):
    if task_id not in validation["task_skill_ids"]:
        raise WorkError(ExitCode.CONTRACT, "handoff_task_not_found", "The explicitly selected TASK does not exist in the formal specification.", {"task_id": task_id})
    return validation["task_skill_ids"][task_id]


def _read_task_source_plan(project_root, task, validation, user_config_root, skill_roots):
    plan, checked, resolved, raw = _read_validated_plan(project_root, task["artifacts"]["plan"], user_config_root, skill_roots)
    if checked["plan_sha256"] != validation["source_plan_sha256"] or plan["artifacts"] != task["artifacts"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_changed", "The source Plan no longer matches the validated TASK.")
    return plan, resolved, raw


def _task_affected_ids(plan, task, task_id):
    known_ids = _plan_item_ids(plan) | {entry["id"] for entry in task["tasks"]} | {entry["id"] for entry in task.get("decisions", [])}
    if task_id is not None:
        selected = next(entry for entry in task["tasks"] if entry["id"] == task_id)
        # Local IDs may be repeated across TASKs; use only the selected TASK.
        for group in ("steps", "validations", "commands", "operations", "files", "inputs", "decisions", "risks"):
            known_ids.update(item["id"] for item in selected.get(group, []))
    return known_ids


def _execute_return_request(request, direction):
    if direction not in {"execute_to_task", "execute_to_plan"}:
        raise WorkError(ExitCode.CONTRACT, "invalid_handoff_direction", "An Execute return direction is required.")
    payload = dict(strict_keys(request, location="handoff_request", required={"summary", "reason", *RETURN_FIELDS}))
    reason = payload.pop("reason")
    return payload, reason


def _execute_skill_fingerprint(plan, skill_id):
    selected_skills = [skill for skill in plan["skill_selection"]["skills"] if skill["id"] == skill_id]
    return selection_sha256("base_only" if skill_id is None else "external_skills", selected_skills)


def _require_no_execution_transaction(execution_path):
    if any(execution_path.glob(".work-*.tmp")):
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_execution_recovery_required", "An execution transaction requires recovery.")


def _execute_return_contract(project_root, direction, task, validation, task_id, skill_id, fingerprint, context, payload, plan):
    contract = _build_handoff(project_root, direction, validation["requirement_id"], task["artifacts"], {
        "task_spec_id": validation["spec_id"], "task_id": task_id,
        "task_sha256": validation["task_sha256"],
        "task_instructions_sha256": validation["task_instructions_sha256"][task_id],
        "skill_id": skill_id, "execute_skill_selection_sha256": fingerprint,
        "execution_context": context,
    }, payload)
    _require_known_affected_ids(contract, _task_affected_ids(plan, task, task_id), code="handoff_unknown_affected_ids")
    return contract


def _require_unstarted_task(project_root, execution, index, row):
    if "lock" in index or row["status"] != "pending" or "latest_attempt" in row:
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_task_already_started", "Preflight return requires an unlocked, initial pending TASK without Attempt history.")
    task_directory = validate_execution_task_layout(project_root, f"{execution}/{row['id']}")
    if task_directory.exists() and (not task_directory.is_dir() or any(task_directory.iterdir())):
        raise WorkError(ExitCode.WORKFLOW_STATE, "handoff_attempt_artifacts_present", "The target TASK execution directory must be absent or empty.")


def build_preflight_return_handoff(
    project_root: Path, request: object, *, direction: str, task_path: str,
    task_id: str, user_config_root: str, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Describe a preflight specification defect, without claiming readiness."""
    payload, reason = _execute_return_request(request, direction)
    task, validation, resolved, raw = _read_validated_task(
        project_root, task_path, user_config_root, skill_roots, validate_file_state=False,
    )
    skill_id = _task_skill_id(validation, task_id)
    plan, plan_resolved, plan_raw = _read_task_source_plan(project_root, task, validation, user_config_root, skill_roots)
    execution = task["artifacts"]["execution"]
    _, execution_path = resolve_project_relative_path(project_root, execution, field="execution_dir")
    _require_no_execution_transaction(execution_path)
    index_relative = f"{execution}/index.json"
    _, index_path = resolve_project_relative_path(project_root, index_relative, field="execution_index")
    index_raw = read_raw(index_path)
    validate_execution_index(index_raw, source=str(index_path))
    index = parse_json_contract(index_raw, source=str(index_path))
    row = _require_index_identity(index, task, validation)[task_id]
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
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Derive all machine fields; the caller supplies summary and affected IDs.

    A valid result describes the current formal Plan, not authorization to
    create TASK artifacts, resume discussion, or execute work.
    """
    payload = strict_keys(request, location="handoff_request", required={"summary", "affected_ids"})
    plan, validation, resolved, raw = _read_validated_plan(project_root, plan_path, user_config_root, skill_roots)
    contract = _build_handoff(project_root, "plan_to_task", validation["requirement_id"], plan["artifacts"], {
        "plan_sha256": validation["plan_sha256"], "skill_selection_sha256": validation["skill_selection_sha256"],
    }, payload)
    _require_known_affected_ids(contract, _plan_item_ids(plan), code="handoff_unknown_plan_ids")
    _require_unchanged_source(project_root, plan_path, resolved, raw, field="plan_path")
    return contract


def _require_matching_handoff_source(contract, expected):
    mismatches = [field for field in ("requirement_id", "artifacts", "source") if contract[field] != expected[field]]
    if mismatches:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_source_mismatch", "The incoming handoff does not match the selected current source.", {"fields": mismatches})


def verify_plan_to_task_handoff(
    project_root: Path, contract: dict[str, object], *, plan_path: str,
    user_config_root: str, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Bind an incoming handoff to the explicitly selected current formal Plan."""
    result = validate_handoff_contract(contract, project_root=project_root)
    if contract["direction"] != "plan_to_task":
        raise WorkError(ExitCode.CONTRACT, "handoff_direction_mismatch", "This receiver requires a Plan-to-Task handoff.")
    expected = build_plan_to_task_handoff(
        project_root, {"summary": contract["summary"], "affected_ids": contract["affected_ids"]},
        plan_path=plan_path, user_config_root=user_config_root, skill_roots=skill_roots,
    )
    _require_matching_handoff_source(contract, expected)
    return {
        **result, "schema": "work-handoff-source-validation/v1",
        "plan_path": expected["artifacts"]["plan"], "source": copy.deepcopy(expected["source"]),
    }


def build_task_to_execute_handoff(
    project_root: Path, request: object, *, task_path: str, task_id: str,
    user_config_root: str, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Describe one explicitly selected formal TASK without performing preflight."""
    payload = strict_keys(request, location="handoff_request", required={"summary"})
    task, validation, resolved, raw = _read_validated_task(project_root, task_path, user_config_root, skill_roots)
    skill_id = _task_skill_id(validation, task_id)
    plan_path = task["artifacts"]["plan"]
    _, plan_resolved, plan_raw = _read_task_source_plan(project_root, task, validation, user_config_root, skill_roots)
    contract = _build_handoff(project_root, "task_to_execute", validation["requirement_id"], task["artifacts"], {
        "task_spec_id": validation["spec_id"], "task_id": task_id,
        "task_sha256": validation["task_sha256"],
        "task_instructions_sha256": validation["task_instructions_sha256"][task_id],
        "skill_id": skill_id,
        "skill_selection_sha256": validation["skill_selection_sha256"],
    }, payload)
    _require_unchanged_source(project_root, task_path, resolved, raw, field="task_path")
    _require_unchanged_source(project_root, plan_path, plan_resolved, plan_raw, field="plan_path")
    return contract


def verify_task_to_execute_handoff(
    project_root: Path, contract: dict[str, object], *, task_path: str, task_id: str,
    user_config_root: str, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Bind an incoming handoff to one explicitly selected current formal TASK."""
    result = validate_handoff_contract(contract, project_root=project_root)
    if contract["direction"] != "task_to_execute":
        raise WorkError(ExitCode.CONTRACT, "handoff_direction_mismatch", "This receiver requires a Task-to-Execute handoff.")
    expected = build_task_to_execute_handoff(
        project_root, {"summary": contract["summary"]}, task_path=task_path, task_id=task_id,
        user_config_root=user_config_root, skill_roots=skill_roots,
    )
    _require_matching_handoff_source(contract, expected)
    return {
        **result, "schema": "work-handoff-source-validation/v1",
        "task_path": expected["artifacts"]["task"], "source": copy.deepcopy(expected["source"]),
    }


def build_task_to_plan_handoff(
    project_root: Path, request: object, *, task_path: str, user_config_root: str,
    task_id: str | None = None, skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Return a formal specification to Plan; semantic decisions remain explicit."""
    payload = strict_keys(request, location="handoff_request", required={"summary", *RETURN_FIELDS})
    task, validation, resolved, raw = _read_validated_task(project_root, task_path, user_config_root, skill_roots)
    source = {
        "plan_sha256": validation["source_plan_sha256"], "task_spec_id": validation["spec_id"],
        "skill_selection_sha256": validation["skill_selection_sha256"],
    }
    if task_id is not None:
        source.update(task_id=task_id, skill_id=_task_skill_id(validation, task_id))
    plan, plan_resolved, plan_raw = _read_task_source_plan(project_root, task, validation, user_config_root, skill_roots)
    contract = _build_handoff(project_root, "task_to_plan", validation["requirement_id"], task["artifacts"], source, payload)
    _require_known_affected_ids(contract, _task_affected_ids(plan, task, task_id), code="handoff_unknown_affected_ids")
    _require_unchanged_source(project_root, task_path, resolved, raw, field="task_path")
    _require_unchanged_source(project_root, task["artifacts"]["plan"], plan_resolved, plan_raw, field="plan_path")
    return contract


def build_execute_return_handoff(
    project_root: Path, request: object, *, direction: str, task_path: str,
    task_id: str, attempt_id: str, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Describe a specification defect using the latest closed Attempt."""
    payload, reason = _execute_return_request(request, direction)
    task, validation, resolved, raw = _read_validated_task(
        project_root, task_path, user_config_root, skill_roots, validate_file_state=False,
    )
    skill_id = _task_skill_id(validation, task_id)
    plan, plan_resolved, plan_raw = _read_task_source_plan(project_root, task, validation, user_config_root, skill_roots)
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
    validate_execute_instructions(selected, attempt, operation="handoff")
    execute_fingerprint = _execute_skill_fingerprint(plan, skill_id)
    if (attempt["skill_id"] != skill_id or row["skill_id"] != skill_id
            or index["skill_selection_sha256"] != validation["skill_selection_sha256"]
            or attempt["execute_skill_selection_sha256"] != execute_fingerprint):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "handoff_skill_identity_mismatch", "Execution skill bindings or fingerprints do not match the validated source.")
    contract = _execute_return_contract(
        project_root, direction, task, validation, task_id, skill_id, execute_fingerprint,
        {"attempt": {"status": attempt["status"], "id": attempt_id},
         "phase": "execution", "issue_type": "specification_defect", "reason": reason}, payload, plan,
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
