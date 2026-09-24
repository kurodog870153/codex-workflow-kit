from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.hierarchy import HierarchyContract
from ...models.instruction import (
    SourceImpactContract, SourceRefreshBatchPreviewContract,
    SourceRefreshBatchPublicationContract, SourceRefreshPreviewContract,
    SourceRefreshPublicationContract,
)
from ...models.task_collection import TaskIndexContract
from ...services.attempt.validation import render_execution_index, validate_execution_index
from ...services.instruction.refresh import (
    canonical_json_sha256, canonical_sha256, default_artifact_paths,
    parse_json_contract, raw_sha256, render_json_contract, replace_journal,
    storage_path, write_exclusive,
)
from ...services.instruction.selection import build_instruction_selection
from ...services.instruction.source import load_instruction_sources
from ...services.instruction.catalog import build_instruction_catalog
from ...services.instruction.task_selection import build_task_document_instruction_selection
from ...services.instruction.work_selection import build_work_instruction_selection
from ...services.specification import transaction as transactions
from ...services.specification.history import execution_history_fingerprints
from ...services.specification.transaction import encode_snapshot, require_no_spec_update, transaction_approval_sha256
from ...services.specification.writer_lock import require_idle_writer, state_writer
from ...services.task.collection_validation import collection_fingerprint_sha256
from ...services.task.document import render_ordered_task_contract
from ...services.task.index_validation import render_task_index_contract as render_ordered_task_index_contract
from ...services.task.item_validation import render_task_item_contract as render_ordered_task_item_contract
from ...services.task.ordering import order_task_contract, order_task_index_contract, order_task_item_contract
from ...services.plan.validation import render_plan_contract
from ...services.instruction.validation_session import ValidationSession
from ...services.workflow.routing import RoutingSourceSession
from .migration import _manifest


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _hierarchy(mode: str, selection: dict[str, Any]) -> HierarchyContract:
    resolved = tuple(selection["resolved_paths"])
    return HierarchyContract(
        schema="work-hierarchy/v1", work_directory=mode,
        selected_paths=tuple(selection["selected_paths"]), resolved_paths=resolved,
        required_paths=resolved, optional_paths=(),
    )


def _current(skill_root: Path, mode: str, selection: dict[str, Any], *,
             session: ValidationSession | None = None):
    hierarchy = _hierarchy(mode, selection)
    references = list(selection["references"])
    if session is not None:
        return session.sources(mode, hierarchy, references)
    return load_instruction_sources(skill_root, mode, hierarchy, references)


def _compatibility(stored: dict[str, Any], current_sources: list[dict[str, Any]]) -> tuple[str, list[str]]:
    old = {(row["kind"], row["logical_name"]): row for row in stored["sources"]}
    new = {(row["kind"], row["logical_name"]): row for row in current_sources}
    if set(old) != set(new):
        return "REVIEW_REQUIRED", sorted(name for _, name in set(old) ^ set(new))
    changed: list[str] = []
    for identity in sorted(old):
        before, after = old[identity], new[identity]
        if before["canonical_sha256"] == after["canonical_sha256"]:
            if before.get("compatibility_revision") is None:
                changed.append(identity[1])
            continue
        changed.append(identity[1])
        revision = before.get("compatibility_revision")
        if revision is None or revision != after["compatibility_revision"]:
            return "REVIEW_REQUIRED", changed
    return ("REFRESHABLE" if changed else "VALID"), changed


def _routing_compatibility(
    stored: dict[str, Any] | None, current: dict[str, Any],
) -> tuple[str, list[str]]:
    if stored is None:
        return "REFRESHABLE", ["routing_manifest"]
    if stored.get("router_compatibility_revision") != current["router_compatibility_revision"]:
        return "REVIEW_REQUIRED", ["routing_manifest"]
    old = {row["logical_name"]: row for row in stored["sources"]}
    new = {row["logical_name"]: row for row in current["sources"]}
    if set(old) != set(new):
        return "REVIEW_REQUIRED", sorted(set(old) ^ set(new))
    changed = []
    for name in sorted(old):
        if old[name]["compatibility_revision"] != new[name]["compatibility_revision"]:
            return "REVIEW_REQUIRED", [name]
        if old[name]["canonical_sha256"] != new[name]["canonical_sha256"]:
            changed.append(name)
    if stored.get("selection_sha256") != current["selection_sha256"] and not changed:
        return "REVIEW_REQUIRED", ["routing_manifest"]
    return ("REFRESHABLE" if changed else "VALID"), changed


def _combined_compatibility(
    stored: dict[str, Any], current_sources: list[dict[str, Any]],
    current_manifest: dict[str, Any],
) -> tuple[str, list[str]]:
    source_state, source_changed = _compatibility(stored, current_sources)
    routing_state, routing_changed = _routing_compatibility(
        stored.get("routing_manifest"), current_manifest,
    )
    changed = sorted(set(source_changed) | set(routing_changed))
    if "REVIEW_REQUIRED" in {source_state, routing_state}:
        return "REVIEW_REQUIRED", changed
    if "REFRESHABLE" in {source_state, routing_state}:
        return "REFRESHABLE", changed
    return "VALID", changed


def _read_json(path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = path.read_bytes()
    return raw, parse_json_contract(raw, source=str(path))


def _render_task_contract(contract: dict[str, Any]) -> bytes:
    return render_ordered_task_contract(order_task_contract(contract))


def _render_task_index_contract(contract: dict[str, Any]) -> bytes:
    checked = TaskIndexContract.model_validate(contract).to_canonical_dict()
    return render_ordered_task_index_contract(
        checked,
        ordered_contract=order_task_index_contract(checked),
    )


def _render_task_item_contract(contract: dict[str, Any]) -> bytes:
    return render_ordered_task_item_contract(
        contract,
        ordered_contract=order_task_item_contract(contract),
    )


def _discover_requirements(project_root: Path) -> dict[str, dict[str, str]]:
    discovered: dict[str, dict[str, str]] = {}
    root = project_root / "outputs" / "work"
    for path in sorted(root.rglob("*.json")) if root.is_dir() else []:
        try:
            value = parse_json_contract(path.read_bytes(), source=str(path))
        except (OSError, WorkError):
            continue
        if value.get("schema") != "work-plan/v1" or not isinstance(value.get("requirement_id"), str):
            continue
        artifacts = value.get("artifacts")
        if not isinstance(artifacts, dict) or not all(isinstance(artifacts.get(name), str) for name in ("plan", "task", "execution")):
            continue
        relative = path.relative_to(project_root).as_posix()
        if artifacts["plan"] != relative:
            continue
        requirement_id = value["requirement_id"]
        if requirement_id in discovered:
            _fail("source_impact_duplicate_requirement", "Multiple Plans declare the same requirement ID.", requirement_id=requirement_id)
        discovered[requirement_id] = {name: artifacts[name] for name in ("plan", "task", "execution")}
    return discovered


def _build_requirement(project_root: Path, skill_root: Path, requirement_id: str, *,
                       artifacts: dict[str, str] | None = None) -> dict[str, Any]:
    if artifacts is None:
        artifacts = _discover_requirements(project_root).get(
            requirement_id, default_artifact_paths(project_root, requirement_id),
        )
    plan_path = storage_path(project_root, artifacts["plan"])
    if not plan_path.is_file():
        _fail("source_refresh_plan_missing", "The requirement Plan does not exist.", requirement_id=requirement_id)
    before: dict[str, bytes] = {}
    after: dict[str, bytes] = {}
    blocked: list[dict[str, Any]] = []
    session = ValidationSession(skill_root, build_catalog=build_instruction_catalog,
                                load_sources=load_instruction_sources)
    routing_sources = RoutingSourceSession(skill_root)
    changed_sources: set[str] = set()
    counts = {"plans": 0, "task_items": 0, "task_indexes": 0, "execution_indexes": 0}

    plan_raw, plan = _read_json(plan_path)
    plan_selection = plan["work_instruction_selection"]
    plan_loaded = _current(skill_root, "plan", plan_selection, session=session)
    plan_current = build_work_instruction_selection(plan_loaded)
    plan_current["routing_manifest"] = _manifest(
        skill_root, mode="plan", status="plan_confirmed", operation="prepare_plan", raw=plan_raw,
        routing_sources=routing_sources, artifact=plan,
    )
    state, changed = _combined_compatibility(
        plan_selection, plan_current["sources"], plan_current["routing_manifest"],
    )
    changed_sources.update(changed)
    if state == "REVIEW_REQUIRED":
        blocked.append({"path": artifacts["plan"], "reason": "compatibility_revision_changed"})
    elif state == "REFRESHABLE":
        plan = copy.deepcopy(plan)
        plan["work_instruction_selection"] = plan_current
        after[artifacts["plan"]] = render_plan_contract(plan)
        before[artifacts["plan"]] = plan_raw
        counts["plans"] = 1
    effective_plan_raw = after.get(artifacts["plan"], plan_raw)

    task_path = storage_path(project_root, artifacts["task"])
    task_loaded_sets = []
    item_contracts: dict[str, dict[str, Any]] = {}
    item_paths: dict[str, str] = {}
    if task_path.is_file():
        index_raw, index = _read_json(task_path)
        base = artifacts["task"].rsplit("/", 1)[0]
        for reference in index["tasks"]:
            relative = base + "/" + reference["path"]
            raw, item = _read_json(storage_path(project_root, relative))
            loaded = _current(skill_root, "task", item["instruction_selection"], session=session)
            task_loaded_sets.append(loaded)
            current = build_instruction_selection(loaded)
            current["routing_manifest"] = _manifest(
                skill_root, mode="task", status="task_confirmed", operation="choose_task", raw=raw,
                routing_sources=routing_sources, artifact=item,
            )
            item_state, item_changed = _combined_compatibility(
                item["instruction_selection"], current["sources"], current["routing_manifest"],
            )
            changed_sources.update(item_changed)
            if item_state == "REVIEW_REQUIRED":
                blocked.append({"path": relative, "reason": "compatibility_revision_changed"})
            elif item_state == "REFRESHABLE":
                item = copy.deepcopy(item)
                item["instruction_selection"] = current
                before[relative] = raw
                after[relative] = _render_task_item_contract(item)
                counts["task_items"] += 1
            item_contracts[reference["id"]] = item
            item_paths[reference["id"]] = relative

        index_current = build_task_document_instruction_selection(task_loaded_sets)
        index_current["routing_manifest"] = _manifest(
            skill_root, mode="task", status="task_confirmed", operation="confirm_review", raw=index_raw,
            routing_sources=routing_sources, artifact=index,
        )
        index_state, index_changed = _combined_compatibility(
            index["instruction_selection"], index_current["sources"], index_current["routing_manifest"],
        )
        changed_sources.update(index_changed)
        if index_state == "REVIEW_REQUIRED":
            blocked.append({"path": artifacts["task"], "reason": "routing_manifest_changed"})
        if not blocked and (after or index_state == "REFRESHABLE"):
            index = copy.deepcopy(index)
            index["source_plan"]["canonical_sha256"] = canonical_sha256(effective_plan_raw, source=artifacts["plan"])
            index["instruction_selection"] = index_current
            for reference in index["tasks"]:
                raw = after.get(item_paths[reference["id"]], storage_path(project_root, item_paths[reference["id"]]).read_bytes())
                reference["canonical_sha256"] = canonical_sha256(raw, source=item_paths[reference["id"]])
            before[artifacts["task"]] = index_raw
            after[artifacts["task"]] = _render_task_index_contract(index)
            counts["task_indexes"] = 1

            execution_relative = artifacts["execution"] + "/index.json"
            execution_path = storage_path(project_root, execution_relative)
            if execution_path.is_file():
                execution_raw, execution = _read_json(execution_path)
                validate_execution_index(execution_raw, source=execution_relative)
                if "lock" in execution or any(row.get("status") == "in_progress" for row in execution["tasks"]):
                    blocked.append({"path": execution_relative, "reason": "active_attempt_snapshot"})
                else:
                    index_raw_new = after[artifacts["task"]]
                    execution = copy.deepcopy(execution)
                    execution["instruction_selection_manifest"] = _manifest(
                        skill_root, mode="execute", status="execution_bound",
                        operation="select_task_for_execution", raw=execution_raw,
                        routing_sources=routing_sources, artifact=execution,
                    )
                    execution["task_index_sha256"] = canonical_sha256(index_raw_new, source=artifacts["task"])
                    execution["task_collection_sha256"] = collection_fingerprint_sha256(
                        execution["task_index_sha256"], index["tasks"],
                    )
                    execution["task_instructions_sha256"] = index["instruction_selection"]["instructions_sha256"]
                    rows = {row["id"]: row for row in execution["tasks"]}
                    for reference in index["tasks"]:
                        row = rows[reference["id"]]
                        row["task_item_sha256"] = reference["canonical_sha256"]
                        row["instructions_sha256"] = item_contracts[reference["id"]]["instruction_selection"]["instructions_sha256"]
                    before[execution_relative] = execution_raw
                    after[execution_relative] = render_execution_index(execution)
                    counts["execution_indexes"] = 1

    if blocked:
        before.clear(); after.clear()
        counts = {key: 0 for key in counts}
    files = [
        {"path": path, "before_sha256": raw_sha256(before[path]), "after_sha256": raw_sha256(after[path])}
        for path in sorted(after)
    ]
    evidence = {
        "requirement_id": requirement_id, "changed_sources": sorted(changed_sources),
        "blocked": blocked, "files": files,
    }
    status = "review_required" if blocked else "refreshable" if files else "valid"
    preview = SourceRefreshPreviewContract.model_validate({
        "schema": "work-source-refresh-preview/v1", "status": status,
        "requirement_id": requirement_id, "changed_sources": len(changed_sources),
        "affected": counts, "blocked": blocked, "files": files,
        "approved_sha256": canonical_json_sha256(evidence),
    }).to_canonical_dict()
    routing_sources.recheck()
    session.recheck()
    return {"preview": preview, "before": before, "after": after, "artifacts": artifacts,
            "changed_source_names": changed_sources}


def source_impact(project_root: Path, skill_root: Path) -> dict[str, object]:
    requirements = []
    changed_names: set[str] = set()
    discovered = _discover_requirements(project_root)
    for requirement_id in sorted(discovered):
        built = _build_requirement(project_root, skill_root, requirement_id,
                                   artifacts=discovered[requirement_id])
        requirements.append(built["preview"])
        changed_names.update(built["changed_source_names"])
    changed = len(changed_names)
    affected = sum(len(row["files"]) for row in requirements)
    affected_requirements = sum(row["status"] != "valid" for row in requirements)
    blocked = sum(len(row["blocked"]) for row in requirements)
    status = "review_required" if blocked else "changes_detected" if affected else "valid"
    return SourceImpactContract.model_validate({
        "schema": "work-source-impact/v1", "status": status,
        "refreshable": blocked == 0, "changed_sources": changed,
        "affected_requirements": affected_requirements,
        "affected_files": affected, "blocked": blocked,
        "requirements": requirements,
    }).to_canonical_dict()


def preview_source_refresh(project_root: Path, skill_root: Path, requirement_id: str) -> dict[str, object]:
    return _build_requirement(project_root, skill_root, requirement_id)["preview"]


def apply_source_refresh(
    project_root: Path, skill_root: Path, requirement_id: str,
    approved_sha256: str, *, operation: str = "apply",
) -> dict[str, object]:
    artifacts = _discover_requirements(project_root).get(
        requirement_id, default_artifact_paths(project_root, requirement_id),
    )
    execution_dir = artifacts["execution"]
    journal_relative = execution_dir + "/.work-source-refresh-" + approved_sha256[:12].upper() + ".json"
    marker_relative = journal_relative + ".done"
    journal_path = storage_path(project_root, journal_relative)
    marker_path = storage_path(project_root, marker_relative)
    if marker_path.is_file():
        journal_raw = journal_path.read_bytes()
        journal = transactions.validate_spec_transaction(journal_raw, source=journal_relative)
        if not transactions.completion_marker_matches(journal_raw, marker_path.read_bytes()):
            _fail("source_refresh_marker_conflict", "The refresh completion marker does not match its journal.")
        saved = journal["metadata"]["request"]
        if saved != {"kind": "source_refresh", "requirement_id": requirement_id, "preview_fingerprint": approved_sha256}:
            _fail("source_refresh_completed_request_changed", "The completed refresh belongs to a different request.")
        return SourceRefreshPublicationContract.model_validate({
            "schema": "work-source-refresh-publication/v1", "status": "already_completed",
            "requirement_id": requirement_id, "approved_sha256": approved_sha256,
            "transaction_approval_sha256": journal["approval_sha256"], "journal": journal_relative,
            "completion_marker": marker_relative, "updated_files": [row["path"] for row in journal["files"]],
        }).to_canonical_dict()
    if operation == "recover":
        journal = transactions.validate_spec_transaction(journal_path.read_bytes(), source=journal_relative)
        saved = journal["metadata"]["request"]
        if saved != {"kind": "source_refresh", "requirement_id": requirement_id, "preview_fingerprint": approved_sha256}:
            _fail("source_refresh_recovery_request_changed", "Recovery requires the identical refresh request.")
        require_no_spec_update(project_root, execution_dir, ignored_record=journal_relative)
        require_idle_writer(project_root, execution_dir)
        with state_writer(project_root, execution_dir):
            published = transactions.publish_journal(project_root, journal_relative, marker_relative)
        return SourceRefreshPublicationContract.model_validate({
            "schema": "work-source-refresh-publication/v1", "status": "already_completed" if published["status"] == "already_published" else "updated",
            "requirement_id": requirement_id, "approved_sha256": approved_sha256,
            "transaction_approval_sha256": journal["approval_sha256"], "journal": journal_relative,
            "completion_marker": marker_relative, "updated_files": [row["path"] for row in journal["files"]],
        }).to_canonical_dict()
    if operation != "apply":
        _fail("source_refresh_operation", "Use apply or recover for source refresh.")
    built = _build_requirement(project_root, skill_root, requirement_id)
    preview = built["preview"]
    if preview["status"] != "refreshable":
        _fail("source_refresh_not_writable", "The source refresh preview is not writable.")
    if preview["approved_sha256"] != approved_sha256:
        _fail("source_refresh_approval_changed", "The approved source refresh fingerprint changed.")
    before, after, artifacts = built["before"], built["after"], built["artifacts"]
    files = [
        {"phase": 10, "path": path, "operation": "replace", "before": encode_snapshot(before[path]), "after": encode_snapshot(after[path])}
        for path in sorted(after)
    ]
    metadata = {
        "request": {"kind": "source_refresh", "requirement_id": requirement_id, "preview_fingerprint": approved_sha256},
        "artifacts": artifacts, "affected_task_ids": [],
        "history_sha256": execution_history_fingerprints(project_root, execution_dir),
        "source_sha256": {path: raw_sha256(raw) for path, raw in sorted(before.items())},
        "candidate_sha256": {path: raw_sha256(raw) for path, raw in sorted(after.items())},
    }
    approval = transaction_approval_sha256(files, metadata)
    journal = {
        "schema": "work-spec-transaction/v1", "transaction_id": "SOURCE-REFRESH-" + approved_sha256[:12].upper(),
        "approval_sha256": approval, "state": "prepared", "published_count": 0,
        "metadata": metadata, "files": files,
    }
    execution_path = storage_path(project_root, execution_dir)
    execution_path.mkdir(parents=True, exist_ok=True)
    require_no_spec_update(project_root, execution_dir)
    require_idle_writer(project_root, execution_dir)
    with state_writer(project_root, execution_dir):
        transactions.write_journal(storage_path(project_root, journal_relative), journal)
        published = transactions.publish_journal(project_root, journal_relative, marker_relative)
    return SourceRefreshPublicationContract.model_validate({
        "schema": "work-source-refresh-publication/v1",
        "status": "already_completed" if published["status"] == "already_published" else "updated",
        "requirement_id": requirement_id, "approved_sha256": approved_sha256,
        "transaction_approval_sha256": approval, "journal": journal_relative,
        "completion_marker": marker_relative, "updated_files": sorted(after),
    }).to_canonical_dict()


def preview_source_refresh_all(project_root: Path, skill_root: Path) -> dict[str, object]:
    impact = source_impact(project_root, skill_root)
    requirements = impact["requirements"]
    status = "review_required" if any(row["status"] == "review_required" for row in requirements) else (
        "refreshable" if any(row["status"] == "refreshable" for row in requirements) else "valid"
    )
    evidence = [(row["requirement_id"], row["approved_sha256"]) for row in requirements]
    return SourceRefreshBatchPreviewContract.model_validate({
        "schema": "work-source-refresh-batch-preview/v1", "status": status,
        "requirements": requirements, "approved_sha256": canonical_json_sha256(evidence),
    }).to_canonical_dict()


def apply_source_refresh_all(
    project_root: Path, skill_root: Path, approved_sha256: str, *,
    operation: str = "apply",
) -> dict[str, object]:
    record_relative = (
        "outputs/work/transactions/pending/source-refresh-batch/"
        + approved_sha256[:12].upper() + ".json"
    )
    record_path = storage_path(project_root, record_relative)
    if record_path.is_file():
        record = SourceRefreshBatchPublicationContract.parse_json_bytes(
            record_path.read_bytes(), source=record_relative,
        ).to_canonical_dict()
        if record["approved_sha256"] != approved_sha256:
            _fail("source_refresh_batch_record_changed", "The batch record approval does not match.")
        if record["status"] != "in_progress":
            return SourceRefreshBatchPublicationContract.model_validate({
                **record, "status": "already_completed",
            }).to_canonical_dict()
    else:
        if operation == "recover":
            _fail("source_refresh_batch_record_missing", "Batch recovery requires its progress record.")
        if operation != "apply":
            _fail("source_refresh_batch_operation", "Use apply or recover for batch refresh.")
        preview = preview_source_refresh_all(project_root, skill_root)
        if preview["status"] != "refreshable":
            _fail("source_refresh_batch_not_writable", "The batch refresh preview is not writable.")
        if preview["approved_sha256"] != approved_sha256:
            _fail("source_refresh_batch_approval_changed", "The approved batch refresh fingerprint changed.")
        requirements = [
            {"requirement_id": row["requirement_id"], "approved_sha256": row["approved_sha256"]}
            for row in preview["requirements"] if row["status"] == "refreshable"
        ]
        record = SourceRefreshBatchPublicationContract.model_validate({
            "schema": "work-source-refresh-batch-publication/v1",
            "status": "in_progress", "semantics": "recoverable_sequential",
            "approved_sha256": approved_sha256, "record_path": record_relative,
            "requirements": requirements, "completed_requirement_ids": [],
            "publications": [],
        }).to_canonical_dict()
        record_path.parent.mkdir(parents=True, exist_ok=True)
        write_exclusive(record_path, render_json_contract(record))

    publications = list(record["publications"])
    for requirement in record["requirements"][len(publications):]:
        requirement_id = requirement["requirement_id"]
        artifacts = _discover_requirements(project_root).get(
            requirement_id, default_artifact_paths(project_root, requirement_id),
        )
        journal = storage_path(
            project_root,
            artifacts["execution"] + "/.work-source-refresh-"
            + requirement["approved_sha256"][:12].upper() + ".json",
        )
        single_operation = "recover" if journal.is_file() and not Path(str(journal) + ".done").is_file() else "apply"
        publication = apply_source_refresh(
            project_root, skill_root, requirement_id,
            requirement["approved_sha256"], operation=single_operation,
        )
        publications.append(publication)
        record = SourceRefreshBatchPublicationContract.model_validate({
            **record,
            "completed_requirement_ids": [row["requirement_id"] for row in record["requirements"][:len(publications)]],
            "publications": publications,
        }).to_canonical_dict()
        replace_journal(record_path, render_json_contract(record))

    status = "already_completed" if publications and all(
        row["status"] == "already_completed" for row in publications
    ) else "updated"
    record = SourceRefreshBatchPublicationContract.model_validate({
        **record, "status": status,
    }).to_canonical_dict()
    replace_journal(record_path, render_json_contract(record))
    return record


__all__ = [
    "apply_source_refresh", "apply_source_refresh_all", "preview_source_refresh",
    "preview_source_refresh_all", "source_impact",
]
