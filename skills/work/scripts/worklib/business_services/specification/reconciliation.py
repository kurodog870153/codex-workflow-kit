"""Preview and publish specification reconciliation for immutable Attempts."""
from __future__ import annotations

import base64
import json
from pathlib import Path
from typing import Any

from ...models.specification.reconciliation import (
    SpecificationReconciliationLedgerContract,
    SpecificationReconciliationPreviewContract,
    SpecificationReconciliationPreviewRequestContract,
    SpecificationReconciliationPublicationContract,
    SpecificationReconciliationPrepareRequestContract,
)
from ...models.common.errors import ExitCode, WorkError
from ...services.attempt.validation import render_attempt_json_contract
from ...services.specification.document_io import (
    canonical_json_sha256,
    raw_sha256,
    render_json_contract,
)
from ...services.specification.storage import read_raw, resolve_project_relative_path
from ...services.specification.document_io import parse_json_contract
from ...services.specification.storage import write_prepared_output
from ...services.specification import transaction as spec_transactions
from ...services.execution.command import require_idle_writer, state_writer, storage_path
from ...services.specification.transaction import require_no_spec_update
from .migration import preview_specification_migration, publish_specification_migration
from ...services.deviation.validation import deviation_reconciliation_target
from ...services.specification.source_resolution import resolve_reconciliation_attempt


def prepare_specification_reconciliation(
    raw_request: bytes, *, project_root: Path, user_config_root: str, skill_roots=None,
    task_operations=None, validate_task_collection_contract=None, validate_plan_contract=None,
    output_file: str | None = None,
) -> dict[str, object]:
    """Build machine migration evidence from semantic specification edits."""
    request = SpecificationReconciliationPrepareRequestContract.parse_json_bytes(
        raw_request, source="specification reconciliation preparation",
    ).to_canonical_dict()
    attempt_path, attempt = resolve_reconciliation_attempt(
        project_root, request["requirement_id"], request["task_position"], request["attempt_position"])
    deviations = attempt.get("execution_deviations", [])
    positions = request.get("deviation_positions", [])
    if not isinstance(deviations, list) or any(position > len(deviations) for position in positions):
        raise WorkError(ExitCode.CONTRACT, "reconciliation_deviation_position", "A selected deviation position does not exist.")
    deviation_ids = sorted(deviations[position - 1]["deviation_id"] for position in positions)
    migration = None
    if request["choice"] != "retain_only":
        from .workflow import prepare_specification
        semantic = {"schema": "work-spec-prepare-request/v1", "requirement_id": request["requirement_id"],
                    "reason": request["reason"], "edits": request["edits"]}
        prepared = prepare_specification(
            render_json_contract(semantic), project_root=project_root,
            user_config_root=user_config_root, task_operations=task_operations,
            skill_roots=skill_roots,
        )
        transaction = prepared["preview"]["transaction"]
        candidate = prepared["request"]
        artifacts = candidate["plan"]["artifacts"]
        index_path = artifacts["task"]
        execution_path = artifacts["execution"] + "/index.json"
        directory = index_path.rsplit("/", 1)[0]
        source_hashes = transaction["metadata"]["source_sha256"]
        after_bytes = {row["path"]: base64.b64decode(row["after"]["base64"])
                       for row in transaction["files"] if "after" in row}
        execution_raw = after_bytes.get(execution_path)
        if execution_raw is None:
            _, resolved = resolve_project_relative_path(project_root, execution_path, field="execution_path")
            execution_raw = read_raw(resolved)
        documents = [
            {"path": artifacts["plan"], "kind": "plan", "content": candidate["plan"]},
            {"path": index_path, "kind": "task_index", "content": candidate["task_index"]},
            {"path": execution_path, "kind": "execution_index", "content": json.loads(execution_raw)},
        ]
        documents.extend({"path": f"{directory}/tasks/{task_id}.json", "kind": "task_item",
                          "task_id": task_id, "content": item}
                         for task_id, item in sorted(candidate["task_items"].items()))
        migration = {"schema": "work-spec-migration-preview-request/v1",
                     "sources": [{"path": path, "raw_sha256": digest} for path, digest in sorted(source_hashes.items())],
                     "candidates": documents, "semantic_decisions": request.get("semantic_decisions", [])}
    full_request = {"schema": "work-spec-reconciliation-preview-request/v1",
                    "attempt_path": attempt_path, "choice": request["choice"],
                    "deviation_ids": deviation_ids, "migration": migration}
    preview = preview_specification_reconciliation(
        render_json_contract(full_request), project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_plan_contract=validate_plan_contract,
    )
    if output_file is not None:
        write_prepared_output(output_file, full_request)
    return {"request": full_request, "preview": preview, "output_file": output_file}


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _ledger_path(attempt_path: str) -> str:
    return attempt_path.rsplit("/", 1)[0] + "/reconciliation.json"


def _existing_ledger(project_root: Path, ledger_path: str, attempt_path: str) -> dict[str, object]:
    _, resolved = resolve_project_relative_path(project_root, ledger_path, field="ledger_path")
    if not resolved.is_file():
        return {"schema": "work-spec-reconciliation-ledger/v1", "attempt_path": attempt_path, "entries": []}
    return SpecificationReconciliationLedgerContract.parse_json_bytes(
        read_raw(resolved), source=ledger_path,
    ).to_canonical_dict()


def preview_specification_reconciliation(
    raw_request: bytes, *, project_root: Path, user_config_root: str, skill_roots=None,
    validate_task_collection_contract=None,
    validate_plan_contract=None,
) -> dict[str, object]:
    request = SpecificationReconciliationPreviewRequestContract.parse_json_bytes(
        raw_request, source="specification reconciliation preview",
    ).to_canonical_dict()
    attempt_path, resolved = resolve_project_relative_path(
        project_root, request["attempt_path"], field="attempt_path",
    )
    attempt_raw = read_raw(resolved)
    attempt = render_attempt_json_contract(attempt_raw, source=attempt_path, project_root=project_root)
    if attempt["status"] == "in_progress":
        _fail("reconciliation_attempt_open", "Reconciliation requires a closed Attempt.")
    parts = attempt_path.rsplit("/", 3)
    if (len(parts) != 4 or parts[1] != attempt.get("task_id")
            or parts[2] != attempt.get("attempt_id") or parts[3] != "attempt.json"):
        _fail("reconciliation_attempt_identity", "The Attempt path does not match its formal identity.")
    index_path = parts[0] + "/index.json"
    index = parse_json_contract(read_raw(storage_path(project_root, index_path)), source=index_path)
    rows = index.get("tasks") if isinstance(index, dict) else None
    matching = [row for row in rows if isinstance(row, dict) and row.get("id") == attempt["task_id"]] if isinstance(rows, list) else []
    if (index.get("schema") != "work-execution-index/v1" or len(matching) != 1
            or matching[0].get("latest_attempt") != attempt["attempt_id"]
            or index.get("task_spec_id") != attempt.get("task_spec_id")):
        _fail("reconciliation_attempt_not_latest", "The closed Attempt is no longer the latest verified execution source.")
    ledger_path = _ledger_path(attempt_path)
    existing_ledger = _existing_ledger(project_root, ledger_path, attempt_path)
    recorded_ids = {entry["deviation_id"] for entry in existing_ledger["entries"]}
    deviations = [
        row for row in attempt.get("execution_deviations", [])
        if row["decision"]["outcome"] == "approved" and row["reconciliation_status"] == "pending"
        and row["deviation_id"] not in recorded_ids
    ]
    pending = [row["deviation_id"] for row in deviations]
    if not pending:
        _fail("reconciliation_nothing_pending", "The Attempt has no approved pending deviations.")
    if request["choice"] == "all":
        selected = pending
    elif request["choice"] == "selective":
        selected = request["deviation_ids"]
        unknown = sorted(set(selected) - set(pending))
        if unknown:
            _fail("reconciliation_unknown_deviation", "Selected deviations are not approved and pending.", ids=unknown)
    else:
        selected = []
    retained = [item for item in pending if item not in selected]
    by_id = {row["deviation_id"]: row for row in deviations}
    classifications = {
        deviation_id: (
            deviation_reconciliation_target(by_id[deviation_id]["proposal"])
            if deviation_id in selected else "retain_only"
        )
        for deviation_id in pending
    }
    migration_preview = None
    if request["migration"] is not None:
        migration_preview = preview_specification_migration(
            render_json_contract(request["migration"]), project_root=project_root,
            user_config_root=user_config_root, skill_roots=skill_roots,
            validate_task_collection_contract=validate_task_collection_contract,
            validate_plan_contract=validate_plan_contract,
        )
    evidence: dict[str, Any] = {
        "request": request, "attempt_sha256": raw_sha256(attempt_raw),
        "pending_deviation_ids": pending, "selected_deviation_ids": selected,
        "retained_deviation_ids": retained,
        "deviation_classifications": classifications,
        "migration_fingerprint": None if migration_preview is None else migration_preview["fingerprint"],
    }
    fingerprint = canonical_json_sha256(evidence)
    new_entries = [
        {"deviation_id": deviation_id,
         "outcome": "incorporated" if deviation_id in selected else "retained",
         "target": classifications[deviation_id],
         "attempt_sha256": raw_sha256(attempt_raw),
         "reconciliation_fingerprint": fingerprint}
        for deviation_id in pending
    ]
    ledger = SpecificationReconciliationLedgerContract.model_validate({
        **existing_ledger, "entries": [*existing_ledger["entries"], *new_entries],
    }).to_canonical_dict()
    ready = migration_preview is None or migration_preview["writable_ready"]
    return SpecificationReconciliationPreviewContract.model_validate({
        "schema": "work-spec-reconciliation-preview/v1",
        "status": "ready" if ready else "blocked", "attempt_path": attempt_path,
        "attempt_sha256": raw_sha256(attempt_raw), "choice": request["choice"],
        "pending_deviation_ids": pending, "selected_deviation_ids": selected,
        "retained_deviation_ids": retained, "migration_preview": migration_preview,
        "deviation_classifications": classifications,
        "ledger_path": ledger_path, "ledger": ledger, "fingerprint": fingerprint,
        "publication_required": True,
        "publication_ready": ready,
    }).to_canonical_dict()


def _publish_ledger_only(
    *, project_root: Path, preview: dict[str, Any]
) -> dict[str, object]:
    ledger_path = preview["ledger_path"]
    target = storage_path(project_root, ledger_path)
    before = read_raw(target) if target.is_file() else None
    after = render_json_contract(preview["ledger"])
    file_row: dict[str, Any] = {"phase": 10, "path": ledger_path}
    if before is None:
        file_row.update(operation="add", after=spec_transactions.encode_snapshot(after))
    else:
        file_row.update(
            operation="replace",
            before=spec_transactions.encode_snapshot(before),
            after=spec_transactions.encode_snapshot(after),
        )
    metadata = {
        "request": {
            "reconciliation_fingerprint": preview["fingerprint"],
            "attempt_path": preview["attempt_path"],
        },
        "artifacts": {},
        "affected_task_ids": [],
        "history_sha256": {},
        "source_sha256": {} if before is None else {ledger_path: raw_sha256(before)},
        "candidate_sha256": {ledger_path: raw_sha256(after)},
    }
    files = [file_row]
    approval = spec_transactions.transaction_approval_sha256(files, metadata)
    journal = {
        "schema": "work-spec-transaction/v1",
        "transaction_id": spec_transactions.derived_transaction_id("RECONCILIATION", approval),
        "approval_sha256": approval,
        "state": "prepared",
        "published_count": 0,
        "metadata": metadata,
        "files": files,
    }
    execution_dir = preview["attempt_path"].split("/TASK-", 1)[0]
    journal_path = (
        execution_dir + "/.work-spec-migration-"
        + preview["fingerprint"][:12].upper() + ".json"
    )
    marker_path = journal_path + ".done"
    require_no_spec_update(project_root, execution_dir, ignored_record=journal_path)
    require_idle_writer(project_root, execution_dir)
    with state_writer(project_root, execution_dir):
        require_no_spec_update(project_root, execution_dir, ignored_record=journal_path)
        spec_transactions.write_journal(storage_path(project_root, journal_path), journal)
        published = spec_transactions.publish_journal(
            project_root, journal_path, marker_path
        )
    if raw_sha256(read_raw(target)) != raw_sha256(after):
        _fail(
            "reconciliation_ledger_post_write",
            "The installed reconciliation ledger differs from approval.",
        )
    return {
        "schema": "work-spec-migration-publication/v1",
        "status": "updated",
        "fingerprint": preview["fingerprint"],
        "transaction_approval_sha256": approval,
        "journal": journal_path,
        "completion_marker": marker_path,
        "documents": [ledger_path],
        "publication_status": published["status"],
        "validator_results": [],
        "relationship_results": [],
    }


def publish_specification_reconciliation(
    raw_request: bytes, *, approved_sha256: str, project_root: Path,
    user_config_root: str, skill_roots=None, validate_task_collection_contract=None,
    validate_plan_contract=None,
) -> dict[str, object]:
    preview = preview_specification_reconciliation(
        raw_request, project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots,
        validate_task_collection_contract=validate_task_collection_contract,
        validate_plan_contract=validate_plan_contract,
    )
    if preview["fingerprint"] != approved_sha256:
        _fail("reconciliation_approval_changed", "The approved reconciliation fingerprint changed.")
    if not preview["publication_ready"]:
        _fail("reconciliation_not_publishable", "This reconciliation does not have a writable candidate set.")
    request = SpecificationReconciliationPreviewRequestContract.parse_json_bytes(
        raw_request, source="specification reconciliation publication",
    ).to_canonical_dict()
    migration = request["migration"]
    if migration is None:
        publication = _publish_ledger_only(project_root=project_root, preview=preview)
    else:
        publication = publish_specification_migration(
            render_json_contract(migration), project_root=project_root,
            user_config_root=user_config_root, skill_roots=skill_roots,
            operation="apply", approved_sha256=preview["migration_preview"]["fingerprint"],
            validate_task_collection_contract=validate_task_collection_contract,
            validate_plan_contract=validate_plan_contract,
            additional_candidates={preview["ledger_path"]: render_json_contract(preview["ledger"])},
        )
    return SpecificationReconciliationPublicationContract.model_validate({
        "schema": "work-spec-reconciliation-publication/v1", "status": "updated",
        "reconciliation_fingerprint": approved_sha256,
        "attempt_path": preview["attempt_path"], "attempt_sha256": preview["attempt_sha256"],
        "selected_deviation_ids": preview["selected_deviation_ids"],
        "retained_deviation_ids": preview["retained_deviation_ids"],
        "ledger_path": preview["ledger_path"],
        "ledger_sha256": raw_sha256(render_json_contract(preview["ledger"])),
        "publication": publication,
        "deviation_classifications": preview["deviation_classifications"],
    }).to_canonical_dict()
