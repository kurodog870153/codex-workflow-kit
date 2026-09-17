"""Explicit v1 TASK file to v2 TASK collection layout migration."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from .specification import _history
from ..contracts.execution_index import derive_overall_status, render_execution_index, validate_execution_index
from ..contracts.plan import render_plan_contract, validate_plan_contract
from ..contracts.spec_transaction import encode_snapshot, render_spec_transaction, transaction_approval_sha256, validate_spec_transaction
from ..contracts.task import validate_task_contract
from ..contracts.task_collection import validate_task_collection_contract
from ..contracts.task_index import render_task_index_contract
from ..contracts.task_item import render_task_item_contract, validate_task_item_contract
from ..contracts.validation import nonempty_string, sha256, strict_keys
from ..foundation import spec_transactions
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.spec_update import require_idle_writer, require_no_spec_update, state_writer, storage_path


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _request(raw: bytes) -> dict[str, Any]:
    value = parse_json_contract(raw, source="TASK layout migration")
    schema = value.get("schema") if isinstance(value, dict) else None
    if value["schema"] not in {"work-task-layout-preflight-request/v1", "work-task-layout-migration-request/v1", "work-task-layout-verify-request/v1"}:
        _fail("layout_migration_schema", "Use a TASK layout migration request schema.")
    if schema == "work-task-layout-preflight-request/v1":
        value = strict_keys(value, location="layout_migration", required={"schema", "plan_path"})
    elif schema == "work-task-layout-migration-request/v1":
        value = strict_keys(value, location="layout_migration", required={"schema", "plan_path", "expected", "target_task_path", "record"})
    else:
        value = strict_keys(value, location="layout_migration", required={"schema", "plan_path", "record"})
    nonempty_string(value["plan_path"], location="plan_path")
    return value


def _baseline(root: Path, user_root: str, plan_path: str, *, skill_roots=None) -> dict[str, Any]:
    plan_raw = read_raw(storage_path(root, plan_path))
    plan = parse_json_contract(plan_raw, source=plan_path)
    artifacts = strict_keys(plan.get("artifacts"), location="artifacts", required={"plan", "task", "execution"})
    if artifacts["plan"] != plan_path or not artifacts["task"].endswith("/task.json"):
        _fail("layout_migration_source", "Layout migration requires a Plan routed to v1 task.json.")
    validate_plan_contract(plan_raw, source=plan_path, actual_plan_path=plan_path, project_root=root,
                           user_config_root=user_root, skill_roots=skill_roots)
    task_raw = read_raw(storage_path(root, artifacts["task"]))
    task = parse_json_contract(task_raw, source=artifacts["task"])
    validation = validate_task_contract(task_raw, source=artifacts["task"], actual_task_path=artifacts["task"],
                                        project_root=root, user_config_root=user_root, skill_roots=skill_roots,
                                        _source_plan_raw=plan_raw)
    execution_path = artifacts["execution"] + "/index.json"
    execution_raw = read_raw(storage_path(root, execution_path))
    execution = parse_json_contract(execution_raw, source=execution_path)
    validate_execution_index(execution_raw, source=execution_path)
    if "lock" in execution:
        _fail("layout_migration_active_execution", "Layout migration is blocked while execution state is locked.")
    return {"plan": plan, "plan_raw": plan_raw, "task": task, "task_raw": task_raw,
            "task_validation": validation, "execution": execution, "execution_raw": execution_raw,
            "execution_path": execution_path, "artifacts": artifacts}


def _candidate(base: dict[str, Any], *, root: Path, user_root: str, skill_roots=None) -> dict[str, Any]:
    old_artifacts = base["artifacts"]
    index_path = old_artifacts["task"].rsplit("/", 1)[0] + "/index.json"
    plan = copy.deepcopy(base["plan"])
    plan["artifacts"]["task"] = index_path
    plan_raw = render_plan_contract(plan)
    plan_validation = validate_plan_contract(plan_raw, source="layout candidate Plan", actual_plan_path=old_artifacts["plan"],
                                             project_root=root, user_config_root=user_root, skill_roots=skill_roots,
                                             _allow_task_index=True)
    logical = copy.deepcopy(base["task"])
    logical["artifacts"]["task"] = index_path
    logical["source_plan"]["canonical_sha256"] = plan_validation["plan_sha256"]
    items, references = {}, []
    for row in logical.pop("tasks"):
        raw = render_task_item_contract({"schema": "work-task-item/v2", **row})
        checked = validate_task_item_contract(raw, source=row["id"], expected_task_id=row["id"])
        items[row["id"]] = raw
        references.append({"id": row["id"], "path": f"tasks/{row['id']}.json", "canonical_sha256": checked["task_item_sha256"]})
    index = {**logical, "schema": "work-task-index/v2", "tasks": references}
    index_raw = render_task_index_contract(index)
    collection = validate_task_collection_contract(index_raw, items, source=index_path, actual_index_path=index_path,
                                                    project_root=root, user_config_root=user_root, skill_roots=skill_roots,
                                                    validate_file_state=False, _source_plan_raw=plan_raw)
    old_rows = {row["id"]: row for row in base["execution"]["tasks"]}
    rows = []
    for item in collection["logical_contract"]["tasks"]:
        row = copy.deepcopy(old_rows[item["id"]])
        row["task_item_sha256"] = collection["task_item_sha256"][item["id"]]
        rows.append(row)
    execution = copy.deepcopy(base["execution"])
    execution["schema"] = "work-execution-index/v2"
    execution["source_v1_task_sha256"] = base["task_validation"]["task_sha256"]
    execution.pop("task_sha256", None)
    execution["task_collection_sha256"] = collection["task_collection_sha256"]
    execution["task_index_sha256"] = collection["task_index_sha256"]
    execution["tasks"] = rows
    execution["overall_status"] = derive_overall_status([row["status"] for row in rows])
    execution_raw = render_execution_index(execution)
    validate_execution_index(execution_raw, source="layout candidate execution")
    return {"plan_raw": plan_raw, "index_path": index_path, "index_raw": index_raw,
            "items": items, "execution_raw": execution_raw, "collection": collection}


def _prepare(root: Path, user_root: str, plan_path: str, *, skill_roots=None, writer_owned: bool = False) -> dict[str, Any]:
    base = _baseline(root, user_root, plan_path, skill_roots=skill_roots)
    if not writer_owned:
        require_idle_writer(root, base["artifacts"]["execution"])
    require_no_spec_update(root, base["artifacts"]["execution"])
    candidate = _candidate(base, root=root, user_root=user_root, skill_roots=skill_roots)
    directory = candidate["index_path"].rsplit("/", 1)[0]
    targets = {base["artifacts"]["plan"]: candidate["plan_raw"], candidate["index_path"]: candidate["index_raw"],
               base["execution_path"]: candidate["execution_raw"]}
    targets.update({f"{directory}/tasks/{task_id}.json": raw for task_id, raw in candidate["items"].items()})
    for path in targets:
        if path not in {base["artifacts"]["plan"], base["execution_path"]} and storage_path(root, path).exists():
            _fail("layout_migration_target_exists", "A v2 layout target already exists.", path=path)
    source = {base["artifacts"]["plan"]: base["plan_raw"], base["execution_path"]: base["execution_raw"]}
    phases = {base["artifacts"]["plan"]: 50, candidate["index_path"]: 30, base["execution_path"]: 40}
    files = []
    for path in sorted(targets, key=lambda path: (20 if "/tasks/" in path else phases[path], path)):
        before, after = source.get(path), targets[path]
        row = {"phase": 20 if "/tasks/" in path else phases[path], "path": path,
               "operation": "add" if before is None else "replace", "after": encode_snapshot(after)}
        if before is not None: row["before"] = encode_snapshot(before)
        files.append(row)
    expected = {"plan_sha256": raw_sha256(base["plan_raw"]), "task_sha256": raw_sha256(base["task_raw"]),
                "execution_index_sha256": raw_sha256(base["execution_raw"])}
    transaction_id = "TASK-LAYOUT-" + raw_sha256(render_json_contract(expected))[:12].upper()
    record = base["artifacts"]["execution"] + "/.work-task-layout-" + transaction_id + ".json"
    request = {"schema": "work-task-layout-migration-request/v1", "plan_path": plan_path,
               "expected": expected, "target_task_path": candidate["index_path"], "record": record}
    metadata = {"request": request, "artifacts": base["artifacts"], "affected_task_ids": [],
                "history_sha256": _history(root, base["artifacts"]["execution"]),
                "source_sha256": {**{path: raw_sha256(raw) for path, raw in source.items()}, base["artifacts"]["task"]: raw_sha256(base["task_raw"])},
                "candidate_sha256": {path: raw_sha256(raw) for path, raw in targets.items()}}
    transaction = {"schema": "work-spec-transaction/v2", "transaction_id": transaction_id,
                   "approval_sha256": transaction_approval_sha256(files, metadata), "state": "prepared", "published_count": 0,
                   "metadata": metadata, "files": files}
    render_spec_transaction(transaction)
    return {"base": base, "candidate": candidate, "request": request, "transaction": transaction}


def layout_migration(raw: bytes, *, project_root: Path, user_config_root: str, skill_roots=None,
                     operation: str, approved_sha256: str | None = None) -> dict[str, Any]:
    request = _request(raw)
    if operation == "preflight":
        prepared = _prepare(project_root, user_config_root, request["plan_path"], skill_roots=skill_roots)
        return {"schema": "work-task-layout-preflight/v1", "status": "ready", "can_prepare_candidate": True,
                "requirement_id": prepared["base"]["task"]["requirement_id"], "source_task_path": prepared["base"]["artifacts"]["task"],
                "target_task_path": prepared["candidate"]["index_path"]}
    if operation == "prepare":
        prepared = _prepare(project_root, user_config_root, request["plan_path"], skill_roots=skill_roots)
        return {"schema": "work-task-layout-prepare/v1", "request": prepared["request"],
                "preview": {"approved_sha256": prepared["transaction"]["approval_sha256"]}}
    if request["schema"] == "work-task-layout-verify-request/v1":
        journal_relative = request["record"]
        transaction = validate_spec_transaction(read_raw(storage_path(project_root, journal_relative)), source=journal_relative)
        marker = read_raw(storage_path(project_root, journal_relative + ".done"))
        from ..foundation.spec_update import completion_marker_matches
        legacy = transaction["metadata"]["artifacts"]["task"]
        verified = (
            completion_marker_matches(render_spec_transaction(transaction), marker)
            and all(raw_sha256(read_raw(storage_path(project_root, path))) == digest
                    for path, digest in transaction["metadata"]["candidate_sha256"].items())
            and raw_sha256(read_raw(storage_path(project_root, legacy))) == transaction["metadata"]["source_sha256"][legacy]
            and _history(project_root, transaction["metadata"]["artifacts"]["execution"]) == transaction["metadata"]["history_sha256"]
        )
        return {"schema": "work-task-layout-verification/v1", "status": "verified" if verified else "blocked", "verified": verified}
    prepared = _prepare(project_root, user_config_root, request["plan_path"], skill_roots=skill_roots) if operation != "recover" else None
    transaction = prepared["transaction"] if prepared else None
    if operation == "recover":
        journal_relative = request["record"]
        transaction = validate_spec_transaction(read_raw(storage_path(project_root, journal_relative)), source=journal_relative)
        if transaction["metadata"]["request"] != request: _fail("layout_migration_recovery_request", "Recovery requires the identical request.")
    else:
        if prepared["request"] != request: _fail("layout_migration_source_changed", "Layout migration source evidence changed.")
        journal_relative = request["record"]
    result = {"schema": "work-task-layout-migration/v1", "status": "valid", "approved_sha256": transaction["approval_sha256"],
              "record": journal_relative, "target_task_path": transaction["metadata"]["request"]["target_task_path"]}
    if operation == "validate": return result
    sha256(approved_sha256, location="approved_sha256")
    if approved_sha256 != transaction["approval_sha256"]: _fail("layout_migration_approval", "Layout migration approval changed.")
    execution = transaction["metadata"]["artifacts"]["execution"]
    require_no_spec_update(project_root, execution, ignored_record=journal_relative if operation == "recover" else None)
    with state_writer(project_root, execution):
        try:
            if _history(project_root, execution) != transaction["metadata"]["history_sha256"]:
                _fail("layout_migration_history_changed", "Execution history changed before layout publication.")
            legacy = transaction["metadata"]["artifacts"]["task"]
            if raw_sha256(read_raw(storage_path(project_root, legacy))) != transaction["metadata"]["source_sha256"][legacy]:
                _fail("layout_migration_source_changed", "The v1 TASK changed before layout publication.")
            if operation == "apply":
                rechecked = _prepare(project_root, user_config_root, request["plan_path"], skill_roots=skill_roots, writer_owned=True)
                if rechecked["transaction"] != transaction: _fail("layout_migration_source_changed", "Layout migration evidence changed before publication.")
                spec_transactions.write_journal(storage_path(project_root, journal_relative), transaction)
            published = spec_transactions.publish_journal(project_root, journal_relative, journal_relative + ".done")
        except (OSError, WorkError) as error:
            raise WorkError(ExitCode.IO_FAILURE, "layout_migration_interrupted", "Preserve layout migration evidence and recover.",
                            {"recovery_required": True, "record": journal_relative}) from error
    result["status"] = "already_completed" if published["status"] == "already_published" else "recovered" if operation == "recover" else "migrated"
    return result
