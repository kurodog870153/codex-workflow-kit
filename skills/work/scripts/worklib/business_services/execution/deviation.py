"""Prepare and record one reviewed execution deviation."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ...models.execution import (
    ExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract,
    ExecutionDeviationRecordContract,
)
from ...services.attempt.validation import (
    canonicalize_attempt_contract, render_attempt_contract, validate_attempt_file,
)
from ...models.common.validation import ContractValuePolicy
from ...models.common.errors import ExitCode, WorkError
from ...services.execution.command import canonical_json_sha256, raw_sha256, read_raw, normalize_relative_path
from ...services.specification.transaction import require_no_spec_update
from ...services.execution.command import storage_path, require_idle_writer, TransactionErrors, prepare_and_replace
from ...services.deviation.validation import (
    deviation_is_blocking,
    deviation_reconciliation_target,
    validate_deviation_action as _validate_action,
)
from .context import validate_execution_identity
from .instructions import validate_execute_instructions
from ...services.record.sequencing import formal_record_kind
from .recovery import _validate_attempt_bytes, _validate_index_bytes


nonempty_string = ContractValuePolicy.nonempty_string
sha256 = ContractValuePolicy.sha256


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, details or None)


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
    operations=None,
) -> dict[str, object]:
    require_idle_writer(project_root, raw_execution_dir)
    return _prepare_execution_deviation(
        raw, source=source, project_root=project_root,
        user_config_root=user_config_root, raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir, task_id=task_id,
        skill_roots=skill_roots,
        operations=operations,
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
    operations=None,
) -> dict[str, object]:
    proposal = ExecutionDeviationProposalContract.parse_json_bytes(raw, source=source).to_canonical_dict()
    execution = normalize_relative_path(raw_execution_dir, field="execution_dir")
    task_relative = normalize_relative_path(raw_task_path, field="task_path")
    require_no_spec_update(project_root, execution)
    directory = storage_path(project_root, execution)
    if any(directory.glob(".work-*.tmp")):
        _fail("deviation_pending_transaction", "Resolve pending transactions before preparing a deviation.")
    context = operations.load_task_execution_context(project_root, user_config_root, task_relative, task_id, skill_roots=skill_roots)
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
    validate_execute_instructions(task, attempt, operation="deviation_prepare", operations=operations)
    record_kind = formal_record_kind(task, proposal["anchor_record_id"].split("#", 1)[0])
    _validate_action(proposal, task, record_kind)
    if any(read_raw(storage_path(project_root, path)) != content for path, content in observed.items()):
        _fail("deviation_source_changed", "A deviation source changed during preparation.")
    preview = {"schema": "work-execution-deviation-preview/v1", "proposal": proposal,
        "record_kind": record_kind, "action_validation": "passed", "semantic_review": "required",
        "classification": deviation_reconciliation_target(proposal),
        "blocking": deviation_is_blocking(proposal),
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
    authorization_evidence: str,
    project_root: Path, user_config_root: str,
    raw_task_path: str, raw_execution_dir: str, task_id: str, skill_roots=None, operations=None,
) -> dict[str, object]:
    sha256(approved_sha256, location="approved_sha256")
    evidence = nonempty_string(
        authorization_evidence, location="authorization_evidence"
    )
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
    if evidence == attempt["authorization"]["authorization_evidence"]:
        _fail(
            "deviation_record_authorization_evidence_reused",
            "A runtime deviation cannot reuse the original Attempt authorization evidence.",
        )
    deviations = list(attempt.get("execution_deviations", []))
    if any(item["approved_preview_sha256"] == approved_sha256 for item in deviations):
        _fail("deviation_record_duplicate", "This approved deviation is already recorded.")
    preview = _prepare_execution_deviation(
        raw, source=source, project_root=project_root,
        user_config_root=user_config_root, raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir, task_id=task_id,
        skill_roots=skill_roots,
        operations=operations,
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
        "supplemental_authorization": {
            "schema": "work-execution-deviation-authorization/v1",
            "preview_sha256": approved_sha256,
            "action": proposal["action"],
            "modifiable_files": proposal["modifiable_files"],
            "authorization_evidence": evidence,
        },
        "decision": {"outcome": "approved", "evidence": evidence},
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
        "classification": preview["classification"], "blocking": preview["blocking"],
        "record_status": "recorded", "lock_status": "record_reserved",
    }).to_canonical_dict()
