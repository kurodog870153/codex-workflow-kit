"""Read-only inventory and request preparation for a reviewed recovery direction."""
from __future__ import annotations

import re
from pathlib import Path

from .context import validate_execution_identity
from ..services.task_collection import load_task_execution_context
from .recovery import _validate_attempt_bytes, _validate_index_bytes
from ..contracts.recovery_models import (
    ExecutionRecoveryPrepareContract, ExecutionRecoveryPrepareRequestContract,
    ExecutionRecoveryRequestContract,
)
from ..contracts.correction import canonicalize_correction_contract, render_correction_contract
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import normalize_relative_path
from ..foundation.spec_update import require_no_spec_update, storage_path
from ..infrastructure.writer_lock import require_idle_writer


def _fail(code, message):
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message)


def _inventory(root, execution):
    directory = storage_path(root, execution)
    names = sorted(path.name for path in directory.glob(".work-*.tmp"))
    for name in names:
        path = storage_path(root, execution + "/" + name)
        if path.is_symlink() or not path.is_file():
            _fail("recovery_prepare_file_type", "Transaction evidence must be regular files.")
    return names


def prepare_execution_recovery(raw, *, source, project_root: Path, user_config_root: str,
                               raw_task_path: str, raw_execution_dir: str, task_id: str,
                               skill_roots=None):
    value = ExecutionRecoveryPrepareRequestContract.parse_request(
        raw, source=source
    )
    # Validate the explicit recovery direction before constructing any paths.
    request = value.to_recovery_request().to_canonical_dict()
    if not isinstance(task_id, str) or not re.fullmatch(r"TASK-\d{3}", task_id):
        _fail("recovery_prepare_task_id", "Use a canonical TASK-nnn ID.")
    task_relative = normalize_relative_path(raw_task_path, field="task_path")
    execution = normalize_relative_path(raw_execution_dir, field="execution_dir")
    require_idle_writer(project_root, execution)
    require_no_spec_update(project_root, execution)
    observed = {}

    def snapshot(relative):
        raw_bytes = read_raw(storage_path(project_root, relative))
        observed[relative] = raw_bytes
        return raw_bytes

    task_context = load_task_execution_context(
        project_root,
        user_config_root,
        task_relative,
        task_id,
        skill_roots=skill_roots,
    )
    task = task_context["contract"]
    validation = task_context["validation"]
    sources = task_context["sources"]
    assert isinstance(task, dict) and isinstance(validation, dict)
    assert isinstance(sources, dict)
    observed.update(sources)
    if task["artifacts"]["task"] != task_relative or task["artifacts"]["execution"] != execution:
        _fail("recovery_prepare_artifact_paths", "Explicit paths do not match the formal TASK.")
    snapshot(task["artifacts"]["plan"])
    if task_id not in {row["id"] for row in task["tasks"]}:
        _fail("recovery_prepare_task_id", "The TASK ID is not present in the formal TASK.")
    index_relative = execution + "/index.json"
    index = _validate_index_bytes(snapshot(index_relative), source=index_relative)
    attempt_id = request["attempt_id"]
    attempt_relative = f"{execution}/{task_id}/{attempt_id}/attempt.json"
    attempt = _validate_attempt_bytes(snapshot(attempt_relative), project_root=project_root, source=attempt_relative)
    if attempt["attempt_id"] != attempt_id:
        _fail("recovery_prepare_attempt_id", "The Attempt identity differs from the requested path.")
    row = validate_execution_identity(task_contract=task, task_validation=validation,
                                     index=index, attempt=attempt, task_id=task_id)
    files = _inventory(project_root, execution)
    transaction = request["transaction"]
    prefix = f".work-{transaction.replace('_', '-')}-{task_id}-{attempt_id}-"
    if any(not name.startswith(prefix) for name in files):
        _fail("recovery_prepare_mixed_transactions", "Preserve foreign, unknown or attempt-start transactions; do not omit them.")
    lock = index.get("lock")
    if transaction == "correction":
        matches = [re.fullmatch(re.escape(prefix) + r"(CORRECTION-\d{3})-(artifact|lock|index)\.tmp", name) for name in files]
        if not files or any(match is None for match in matches) or len({match[1] for match in matches}) != 1:
            _fail("recovery_prepare_correction_identity", "Exactly one preserved Correction transaction is required.")
        correction_id = attempt_id + "-" + matches[0][1]
        if attempt["status"] == "in_progress" or (lock is not None and (
            lock.get("kind") != "correction" or lock.get("task_id") != task_id
            or lock.get("correction_id") != correction_id
            or lock.get("execute_instructions_sha256") != attempt["execute_instructions_sha256"]
        )):
            _fail("recovery_prepare_lock", "The Correction direction conflicts with the Attempt or lock.")
        correction_relative = f"{execution}/{task_id}/{attempt_id}/corrections/{correction_id}.json"
        if storage_path(project_root, correction_relative).is_file():
            snapshot(correction_relative)
    else:
        expected_lock = {"kind": "execution", "task_id": task_id, "attempt_id": attempt_id,
                         "execute_instructions_sha256": attempt["execute_instructions_sha256"]}
        if row.get("latest_attempt") != attempt_id or row["status"] != "in_progress" or not isinstance(lock, dict) or any(
            lock.get(key) != expected for key, expected in expected_lock.items()
        ):
            _fail("recovery_prepare_lock", "The active index and execution lock must identify the requested Attempt.")
        record_id = lock.get("record_id")
        safe_record = record_id.replace("#", "-retry-") if isinstance(record_id, str) else None
        if transaction == "record_begin":
            if record_id is not None or len(files) != 1 or not re.fullmatch(
                re.escape(prefix) + r"(?:CMD|OP|VAL)-\d{3}(?:-retry-\d+)?\.tmp", files[0]
            ):
                _fail("recovery_prepare_record_identity", "Record-begin requires one prepared record and an unreserved lock.")
        elif transaction in {"command_correction", "record_finish"}:
            expected_files = {prefix + str(safe_record) + suffix for suffix in (
                (".tmp",) if transaction == "command_correction" else ("-attempt.tmp", "-index.tmp")
            )}
            if safe_record is None or not set(files) <= expected_files or (
                transaction == "command_correction" and (not record_id.startswith("CMD-") or "command_correction" in lock)
            ):
                _fail("recovery_prepare_record_identity", "The transaction files do not identify the reserved record.")
        elif record_id is not None or not set(files) <= {prefix + "attempt.tmp", prefix + "index.tmp"}:
            _fail("recovery_prepare_record_identity", "Attempt-close requires its own files and no reserved record.")
        if not files and not (
            transaction == "attempt_close" and attempt["status"] != "in_progress" and record_id is None
            or transaction == "record_finish" and isinstance(record_id, str)
            and any(record["id"] == record_id for record in attempt["records"])
        ):
            _fail("recovery_prepare_missing_evidence", "No preserved files or supported post-write evidence establish this direction.")
    for name in files:
        relative = execution + "/" + name
        preserved = snapshot(relative)
        if name.endswith("-attempt.tmp"):
            _validate_attempt_bytes(preserved, project_root=project_root, source=relative)
        elif transaction == "correction" and name.endswith("-artifact.tmp"):
            correction = canonicalize_correction_contract(parse_json_contract(preserved, source=relative))
            if render_correction_contract(correction) != preserved:
                _fail("recovery_prepare_noncanonical_correction", "The preserved Correction is not canonical.")
        else:
            _validate_index_bytes(preserved, source=relative)
    request["transaction_files"] = files
    ExecutionRecoveryRequestContract.model_validate(request)
    if files != _inventory(project_root, execution) or any(
        read_raw(storage_path(project_root, relative)) != old for relative, old in observed.items()
    ):
        _fail("recovery_prepare_source_changed", "Recovery evidence changed during preparation.")
    require_idle_writer(project_root, execution)
    require_no_spec_update(project_root, execution)
    return ExecutionRecoveryPrepareContract.model_validate({"schema": "work-execution-recovery-prepare/v1", "status": "prepared",
        "request": request, "task_id": task_id, "attempt_path": attempt_relative, "index_path": index_relative,
        "lock": lock, "attempt_status": attempt["status"],
        "evidence": {path: {"raw_sha256": raw_sha256(content), "size_bytes": len(content)}
                     for path, content in observed.items()},
        "recovery_validation": "requires_authorized_recover", "recovery_authorized": False}).to_canonical_dict()
