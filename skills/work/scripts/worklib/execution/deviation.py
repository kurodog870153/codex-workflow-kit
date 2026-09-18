"""Prepare and record one reviewed execution deviation."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ..contracts.execution_deviation_models import (
    ExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract,
    ExecutionDeviationRecordContract,
)
from ..contracts.attempt import (
    canonicalize_attempt_contract, render_attempt_contract, validate_attempt_file,
)
from ..contracts.validation import nonempty_string, sha256
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_json_sha256, raw_sha256, read_raw
from ..foundation.paths import normalize_relative_path
from ..foundation.spec_update import require_no_spec_update, storage_path
from ..infrastructure.writer_lock import require_idle_writer
from ..infrastructure.atomic_replace import TransactionErrors, prepare_and_replace
from ..services.task_collection import load_task_execution_context
from .context import validate_execution_identity
from .instructions import validate_execute_instructions
from .records import formal_record_kind
from .recovery import _validate_attempt_bytes, _validate_index_bytes


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, details or None)


def _formal_ids(task: dict[str, Any]) -> set[str]:
    fields = ("steps", "commands", "operations", "validations", "decisions")
    return {item["id"] for field in fields for item in task.get(field, [])}


def _validate_action(proposal: dict[str, Any], task: dict[str, Any], record_kind: str) -> None:
    action = proposal["action"]
    anchor = proposal["anchor_record_id"]
    base_id = anchor.split("#", 1)[0]
    formal_ids = _formal_ids(task)
    if base_id not in proposal["task_basis"] or any(item not in formal_ids for item in proposal["task_basis"]):
        _fail("deviation_task_basis", "task_basis must contain only formal target TASK IDs and include the anchor base ID.")
    kind = action["kind"]
    if kind == "replace_command":
        if record_kind != "command" or action["record_id"] != anchor:
            _fail("deviation_action_anchor", "replace_command must target the reserved command record.")
    elif kind == "skip_record":
        if action["record_id"] != anchor:
            _fail("deviation_action_anchor", "skip_record must target the reserved record.")
    elif kind == "adjust_operation":
        if record_kind != "operation" or action["operation"]["id"] != base_id:
            _fail("deviation_action_anchor", "adjust_operation must preserve the reserved operation ID.")
    elif kind == "add_command":
        if action["after_record_id"] != anchor or action["command"]["id"] in formal_ids:
            _fail("deviation_action_identity", "add_command must follow the anchor and use a new formal ID.")
    elif kind == "add_validation" and action["validation"]["id"] in formal_ids:
        _fail("deviation_action_identity", "add_validation must use a new formal ID.")


def prepare_execution_deviation(
    raw: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots=None,
) -> dict[str, object]:
    require_idle_writer(project_root, raw_execution_dir)
    return _prepare_execution_deviation(
        raw, source=source, project_root=project_root,
        user_config_root=user_config_root, raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir, task_id=task_id,
        skill_roots=skill_roots,
    )


def _prepare_execution_deviation(
    raw: bytes,
    *,
    source: str,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    skill_roots=None,
) -> dict[str, object]:
    proposal = ExecutionDeviationProposalContract.parse_json_bytes(raw, source=source).to_canonical_dict()
    execution = normalize_relative_path(raw_execution_dir, field="execution_dir")
    task_relative = normalize_relative_path(raw_task_path, field="task_path")
    require_no_spec_update(project_root, execution)
    directory = storage_path(project_root, execution)
    if any(directory.glob(".work-*.tmp")):
        _fail("deviation_pending_transaction", "Resolve pending transactions before preparing a deviation.")
    context = load_task_execution_context(project_root, user_config_root, task_relative, task_id, skill_roots=skill_roots)
    contract, validation, context_sources = context["contract"], context["validation"], context["sources"]
    assert isinstance(contract, dict) and isinstance(validation, dict) and isinstance(context_sources, dict)
    if proposal["task_id"] != task_id:
        _fail("deviation_task_identity", "The proposal TASK ID must match the selected TASK.")
    artifacts = contract["artifacts"]
    if artifacts["task"] != task_relative or artifacts["execution"] != execution:
        _fail("deviation_paths", "Explicit paths must match the formal TASK.")
    observed = dict(context_sources)
    observed[artifacts["plan"]] = read_raw(storage_path(project_root, artifacts["plan"]))
    index_relative = execution + "/index.json"
    index_raw = read_raw(storage_path(project_root, index_relative))
    observed[index_relative] = index_raw
    index = _validate_index_bytes(index_raw, source=index_relative)
    attempt_relative = f"{execution}/{task_id}/{proposal['attempt_id']}/attempt.json"
    attempt_raw = read_raw(storage_path(project_root, attempt_relative))
    observed[attempt_relative] = attempt_raw
    attempt = _validate_attempt_bytes(attempt_raw, project_root=project_root, source=attempt_relative)
    row = validate_execution_identity(task_contract=contract, task_validation=validation,
        index=index, attempt=attempt, task_id=task_id)
    lock = index.get("lock")
    expected = {"kind": "execution", "task_id": task_id, "attempt_id": proposal["attempt_id"],
                "record_id": proposal["anchor_record_id"],
                "execute_instructions_sha256": attempt["execute_instructions_sha256"]}
    if (attempt.get("status") != "in_progress" or row.get("status") != "in_progress"
        or row.get("latest_attempt") != proposal["attempt_id"] or not isinstance(lock, dict)
        or any(lock.get(key) != value for key, value in expected.items())):
        _fail("deviation_active_record", "The proposal must match the active Attempt and reserved record.")
    task = next(item for item in contract["tasks"] if item["id"] == task_id)
    validate_execute_instructions(task, attempt, operation="deviation_prepare")
    record_kind = formal_record_kind(task, proposal["anchor_record_id"].split("#", 1)[0])
    _validate_action(proposal, task, record_kind)
    require_deviation(attempt, proposal["action"])
    if any(read_raw(storage_path(project_root, path)) != content for path, content in observed.items()):
        _fail("deviation_source_changed", "A deviation source changed during preparation.")
    preview = {"schema": "work-execution-deviation-preview/v1", "proposal": proposal,
        "record_kind": record_kind, "action_validation": "passed", "semantic_review": "required",
        "sources": {path: raw_sha256(content) for path, content in observed.items()}}
    preview["preview_sha256"] = canonical_json_sha256(preview)
    return ExecutionDeviationPreviewContract.model_validate(preview).to_canonical_dict()


TRANSACTION_ERRORS = TransactionErrors(
    transaction_present=("deviation_record_transaction_present", "A deviation-record transaction already requires recovery."),
    prepare_failed=("deviation_record_prepare_failed", "The deviation record could not be prepared."),
    source_changed=("deviation_record_source_changed", "The Attempt changed before recording the deviation."),
    replace_failed=("deviation_record_replace_failed", "The prepared deviation record could not be installed."),
    write_mismatch=("deviation_record_write_mismatch", "The installed Attempt differs from the prepared deviation record."),
)


def record_execution_deviation(
    raw: bytes, *, source: str, approved_sha256: str,
    project_root: Path, user_config_root: str,
    raw_task_path: str, raw_execution_dir: str, task_id: str, skill_roots=None,
) -> dict[str, object]:
    sha256(approved_sha256, location="approved_sha256")
    proposal = ExecutionDeviationProposalContract.parse_json_bytes(
        raw, source=source
    ).to_canonical_dict()
    execution = normalize_relative_path(raw_execution_dir, field="execution_dir")
    attempt_id = proposal["attempt_id"]
    attempt_relative = f"{execution}/{task_id}/{attempt_id}/attempt.json"
    attempt_path = storage_path(project_root, attempt_relative)
    attempt_raw = read_raw(attempt_path)
    attempt = _validate_attempt_bytes(
        attempt_raw, project_root=project_root, source=attempt_relative
    )
    manifest_evidence = authorization_evidence(attempt)
    deviations = list(attempt.get("execution_deviations", []))
    if any(item["approved_preview_sha256"] == approved_sha256 for item in deviations):
        _fail("deviation_record_duplicate", "This approved deviation is already recorded.")
    preview = _prepare_execution_deviation(
        raw, source=source, project_root=project_root,
        user_config_root=user_config_root, raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir, task_id=task_id,
        skill_roots=skill_roots,
    )
    if preview["preview_sha256"] != approved_sha256:
        _fail("deviation_record_approval_changed", "Sources or proposal content changed after review.")
    proposal = preview["proposal"]
    if len(deviations) >= 999:
        _fail("deviation_record_limit", "The Attempt cannot allocate another DEVIATION-nnn ID.")
    deviation_id = f"DEVIATION-{len(deviations) + 1:03d}"
    artifact = {
        "schema": "work-execution-deviation/v1",
        "deviation_id": deviation_id,
        "approved_preview_sha256": approved_sha256,
        "proposal": proposal,
        "decision": {"outcome": "approved", "evidence": manifest_evidence},
        "reconciliation_status": "pending",
    }
    candidate = copy.deepcopy(attempt)
    candidate["execution_deviations"] = [*deviations, artifact]
    candidate = canonicalize_attempt_contract(candidate, project_root=project_root)
    rendered = render_attempt_contract(candidate, project_root=project_root)
    safe_record = proposal["anchor_record_id"].replace("#", "-retry-")
    temporary = storage_path(
        project_root,
        f"{execution}/.work-deviation-record-{task_id}-{attempt_id}-{safe_record}-attempt.tmp",
    )
    prepare_and_replace(
        source_path=attempt_path, source_bytes=attempt_raw, target_bytes=rendered,
        temporary_path=temporary, stage="attempt_update_prepared",
        errors=TRANSACTION_ERRORS,
    )
    validate_attempt_file(project_root, attempt_relative)
    return ExecutionDeviationRecordContract.model_validate({
        "schema": "work-execution-deviation-record/v1",
        "task_id": task_id, "attempt_id": attempt_id,
        "deviation_id": deviation_id, "attempt_path": attempt_relative,
        "record_status": "recorded", "lock_status": "record_reserved",
    }).to_canonical_dict()
from .authorization import authorization_evidence, require_deviation
