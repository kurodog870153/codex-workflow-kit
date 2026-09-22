from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.instruction import (
    InstructionMigrationPreviewContract, InstructionMigrationPublicationContract,
)
from ...models.task_collection import TaskIndexContract
from ...services.attempt.validation import render_execution_index, validate_execution_index
from ...services.instruction.refresh import (
    canonical_json_sha256, canonical_sha256, default_artifact_paths,
    parse_json_contract, raw_sha256, storage_path,
)
from ...services.specification import transaction as transactions
from ...services.specification.history import execution_history_fingerprints
from ...services.specification.transaction import (
    encode_snapshot, require_no_spec_update, transaction_approval_sha256,
)
from ...services.specification.writer_lock import require_idle_writer, state_writer
from ...services.task.collection_validation import collection_fingerprint_sha256
from ...services.task.document import render_ordered_task_contract
from ...services.task.index_validation import render_task_index_contract
from ...services.task.item_validation import render_task_item_contract
from ...services.task.ordering import order_task_contract, order_task_index_contract, order_task_item_contract
from ...services.plan.validation import render_plan_contract
from ...services.workflow.routing import ROUTER_COMPATIBILITY_REVISION, build_routing_selection


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _read(path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = path.read_bytes()
    return raw, parse_json_contract(raw, source=str(path))


def _manifest(skill_root: Path, *, mode: str, status: str, operation: str, raw: bytes) -> dict[str, Any]:
    artifact = parse_json_contract(raw, source="migration routing state")
    state = {
        key: artifact[key]
        for key in ("schema", "requirement_id", "spec_id", "task_spec_id", "id", "status", "overall_status")
        if key in artifact
    }
    routing = build_routing_selection(
        skill_root, status=status, next_action=operation, confirmation=False,
        mode=mode, artifact_lifecycle="confirmed",
        authorization_state="authorized", verified_state_sha256=canonical_json_sha256(state),
    )
    if routing["routing_status"] != "VALID":
        _fail("instruction_migration_routing_review_required", "The new router could not build a valid manifest.", status=status, operation=operation)
    return copy.deepcopy(routing["selection_manifest"])


def _render_item(value: dict[str, Any]) -> bytes:
    return render_task_item_contract(value, ordered_contract=order_task_item_contract(value))


def _render_index(value: dict[str, Any]) -> bytes:
    checked = TaskIndexContract.model_validate(value).to_canonical_dict()
    return render_task_index_contract(checked, ordered_contract=order_task_index_contract(checked))


def _build(project_root: Path, skill_root: Path, requirement_id: str) -> dict[str, Any]:
    artifacts = default_artifact_paths(project_root, requirement_id)
    before: dict[str, bytes] = {}
    after: dict[str, bytes] = {}
    excluded: list[dict[str, Any]] = []
    counts = {"plans": 0, "task_items": 0, "task_indexes": 0, "execution_indexes": 0}
    plan_path = storage_path(project_root, artifacts["plan"])
    if not plan_path.is_file():
        _fail("instruction_migration_plan_missing", "The requirement Plan does not exist.")
    plan_raw, plan = _read(plan_path)
    plan = copy.deepcopy(plan)
    plan_manifest = _manifest(skill_root, mode="plan", status="plan_confirmed", operation="prepare_plan", raw=plan_raw)
    if plan["work_instruction_selection"].get("routing_manifest") != plan_manifest:
        plan["work_instruction_selection"]["routing_manifest"] = plan_manifest
        before[artifacts["plan"]] = plan_raw
        after[artifacts["plan"]] = render_plan_contract(plan)
        counts["plans"] = 1
    effective_plan = after.get(artifacts["plan"], plan_raw)

    task_path = storage_path(project_root, artifacts["task"])
    index = None
    items: dict[str, dict[str, Any]] = {}
    item_paths: dict[str, str] = {}
    if task_path.is_file():
        index_raw, index = _read(task_path)
        index = copy.deepcopy(index)
        base = artifacts["task"].rsplit("/", 1)[0]
        for reference in index["tasks"]:
            relative = base + "/" + reference["path"]
            raw, item = _read(storage_path(project_root, relative))
            item = copy.deepcopy(item)
            manifest = _manifest(skill_root, mode="task", status="task_confirmed", operation="choose_task", raw=raw)
            if item["instruction_selection"].get("routing_manifest") != manifest:
                item["instruction_selection"]["routing_manifest"] = manifest
                before[relative] = raw
                after[relative] = _render_item(item)
                counts["task_items"] += 1
            items[reference["id"]] = item
            item_paths[reference["id"]] = relative
        index_manifest = _manifest(skill_root, mode="task", status="task_confirmed", operation="confirm_review", raw=index_raw)
        if index["instruction_selection"].get("routing_manifest") != index_manifest or counts["task_items"] or counts["plans"]:
            index["source_plan"]["canonical_sha256"] = canonical_sha256(effective_plan, source=artifacts["plan"])
            index["instruction_selection"]["routing_manifest"] = index_manifest
            for reference in index["tasks"]:
                item_raw = after.get(item_paths[reference["id"]], storage_path(project_root, item_paths[reference["id"]]).read_bytes())
                reference["canonical_sha256"] = canonical_sha256(item_raw, source=item_paths[reference["id"]])
            before[artifacts["task"]] = index_raw
            after[artifacts["task"]] = _render_index(index)
            counts["task_indexes"] = 1

        execution_relative = artifacts["execution"] + "/index.json"
        execution_path = storage_path(project_root, execution_relative)
        if execution_path.is_file():
            execution_raw, execution = _read(execution_path)
            validate_execution_index(execution_raw, source=execution_relative)
            if execution.get("lock") is not None or any(row.get("status") == "in_progress" for row in execution["tasks"]):
                excluded.append({"path": execution_relative, "reason": "active_attempt_snapshot"})
            else:
                execution = copy.deepcopy(execution)
                execution["instruction_selection_manifest"] = _manifest(
                    skill_root, mode="execute", status="execution_bound",
                    operation="select_task_for_execution", raw=execution_raw,
                )
                if index is not None and artifacts["task"] in after:
                    execution["task_index_sha256"] = canonical_sha256(after[artifacts["task"]], source=artifacts["task"])
                    execution["task_collection_sha256"] = collection_fingerprint_sha256(execution["task_index_sha256"], index["tasks"])
                    for reference in index["tasks"]:
                        row = next(row for row in execution["tasks"] if row["id"] == reference["id"])
                        row["task_item_sha256"] = reference["canonical_sha256"]
                candidate = render_execution_index(execution)
                if candidate != execution_raw:
                    before[execution_relative] = execution_raw
                    after[execution_relative] = candidate
                    counts["execution_indexes"] = 1

    if excluded:
        before.clear(); after.clear()
        counts = {key: 0 for key in counts}
    files = [
        {"path": path, "before_sha256": raw_sha256(before[path]), "after_sha256": raw_sha256(after[path])}
        for path in sorted(after)
    ]
    evidence = {
        "requirement_id": requirement_id, "router_compatibility_revision": ROUTER_COMPATIBILITY_REVISION,
        "excluded": excluded, "files": files,
    }
    status = "review_required" if excluded else "migration_required" if files else "current"
    preview = InstructionMigrationPreviewContract.model_validate({
        "schema": "work-instruction-migration-preview/v1", "status": status,
        "requirement_id": requirement_id,
        "router_compatibility_revision": ROUTER_COMPATIBILITY_REVISION,
        "affected": counts, "excluded": excluded, "files": files,
        "approved_sha256": canonical_json_sha256(evidence),
    }).to_canonical_dict()
    return {"preview": preview, "before": before, "after": after, "artifacts": artifacts}


def preview_instruction_migration(project_root: Path, skill_root: Path, requirement_id: str) -> dict[str, object]:
    return _build(project_root, skill_root, requirement_id)["preview"]


def apply_instruction_migration(project_root: Path, skill_root: Path, requirement_id: str, approved_sha256: str) -> dict[str, object]:
    artifacts = default_artifact_paths(project_root, requirement_id)
    execution_dir = artifacts["execution"]
    journal_relative = execution_dir + "/.work-instruction-migration-" + approved_sha256[:12].upper() + ".json"
    marker_relative = journal_relative + ".done"
    if storage_path(project_root, marker_relative).is_file():
        journal_raw = storage_path(project_root, journal_relative).read_bytes()
        journal = transactions.validate_spec_transaction(journal_raw, source=journal_relative)
        marker_raw = storage_path(project_root, marker_relative).read_bytes()
        if not transactions.completion_marker_matches(journal_raw, marker_raw):
            _fail("instruction_migration_marker_conflict", "The migration completion marker does not match its journal.")
        return InstructionMigrationPublicationContract.model_validate({
            "schema": "work-instruction-migration-publication/v1", "status": "already_completed",
            "requirement_id": requirement_id, "approved_sha256": approved_sha256,
            "transaction_approval_sha256": journal["approval_sha256"], "journal": journal_relative,
            "completion_marker": marker_relative,
            "updated_files": [row["path"] for row in journal["files"]],
        }).to_canonical_dict()
    built = _build(project_root, skill_root, requirement_id)
    preview = built["preview"]
    if preview["status"] != "migration_required":
        _fail("instruction_migration_not_writable", "The instruction migration preview is not writable.")
    if preview["approved_sha256"] != approved_sha256:
        _fail("instruction_migration_approval_changed", "The approved instruction migration fingerprint changed.")
    before, after = built["before"], built["after"]
    files = [
        {"phase": 10, "path": path, "operation": "replace", "before": encode_snapshot(before[path]), "after": encode_snapshot(after[path])}
        for path in sorted(after)
    ]
    metadata = {
        "request": {"kind": "instruction_migration", "requirement_id": requirement_id, "preview_fingerprint": approved_sha256},
        "artifacts": artifacts, "affected_task_ids": [],
        "history_sha256": execution_history_fingerprints(project_root, execution_dir),
        "source_sha256": {path: raw_sha256(raw) for path, raw in sorted(before.items())},
        "candidate_sha256": {path: raw_sha256(raw) for path, raw in sorted(after.items())},
    }
    approval = transaction_approval_sha256(files, metadata)
    journal = {
        "schema": "work-spec-transaction/v1",
        "transaction_id": "INSTRUCTION-MIGRATION-" + approved_sha256[:12].upper(),
        "approval_sha256": approval, "state": "prepared", "published_count": 0,
        "metadata": metadata, "files": files,
    }
    require_no_spec_update(project_root, execution_dir)
    require_idle_writer(project_root, execution_dir)
    with state_writer(project_root, execution_dir):
        transactions.write_journal(storage_path(project_root, journal_relative), journal)
        published = transactions.publish_journal(project_root, journal_relative, marker_relative)
    return InstructionMigrationPublicationContract.model_validate({
        "schema": "work-instruction-migration-publication/v1",
        "status": "already_completed" if published["status"] == "already_published" else "updated",
        "requirement_id": requirement_id, "approved_sha256": approved_sha256,
        "transaction_approval_sha256": approval, "journal": journal_relative,
        "completion_marker": marker_relative, "updated_files": sorted(after),
    }).to_canonical_dict()


__all__ = ["apply_instruction_migration", "preview_instruction_migration"]
