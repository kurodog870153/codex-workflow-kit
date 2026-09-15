"""Explicitly reviewed repair of v2 TASK collections and execution bindings."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from .specification import _history
from .specification_v2 import _execution
from ..contracts.execution_index import render_execution_index, validate_execution_index
from ..contracts.spec_transaction import encode_snapshot, render_spec_transaction, transaction_approval_sha256, validate_spec_transaction
from ..contracts.task_collection import validate_task_collection_contract
from ..contracts.task_diagnostics import diagnose_task_collection
from ..contracts.task_index import render_task_index_contract
from ..contracts.task_item import render_task_item_contract
from ..contracts.validation import nonempty_string, sha256, strict_keys
from ..foundation import spec_transactions
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import resolve_project_relative_path, validate_task_collection_index_path
from ..foundation.spec_update import require_idle_writer, require_no_spec_update, state_writer, storage_path


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _render(value: object) -> bytes:
    return render_json_contract(value)


def _artifacts(root: Path, requirement: str, value: object) -> dict[str, str]:
    artifacts = strict_keys(value, location="artifacts", required={"plan", "task", "execution"})
    validate_task_collection_index_path(root, requirement, artifacts["task"])
    for field in ("plan", "execution"):
        normalized, _ = resolve_project_relative_path(root, artifacts[field], field=field)
        if normalized != artifacts[field]:
            _fail("task_repair_paths", "Repair paths must be normalized project-relative paths.")
    return artifacts


def _candidate(request: dict[str, Any], *, root: Path, user_root: str, skill_roots=None) -> dict[str, Any]:
    index = request["task_index"]
    items = request["task_items"]
    if not isinstance(index, dict) or not isinstance(items, dict) or not items:
        _fail("task_repair_candidate", "V2 repair requires an explicit complete index and TASK item map.")
    index_raw = render_task_index_contract(index)
    item_raw = {task_id: render_task_item_contract(item) for task_id, item in items.items()}
    plan_raw = read_raw(storage_path(root, request["artifacts"]["plan"]))
    validation = validate_task_collection_contract(
        index_raw, item_raw, source=request["artifacts"]["task"],
        actual_index_path=request["artifacts"]["task"], project_root=root,
        user_config_root=user_root, skill_roots=skill_roots,
        validate_file_state=False, _source_plan_raw=plan_raw,
    )
    return {"index_raw": index_raw, "item_raw": item_raw, "validation": validation, "plan_raw": plan_raw}


def _item_paths(root: Path, index_path: str) -> dict[str, bytes]:
    directory = storage_path(root, index_path.rsplit("/", 1)[0] + "/tasks")
    if not directory.exists():
        return {}
    if directory.is_symlink() or not directory.is_dir():
        _fail("task_repair_item_directory", "The TASK item storage is not a safe directory.")
    result: dict[str, bytes] = {}
    for path in directory.iterdir():
        if path.is_symlink() or not path.is_file() or path.suffix != ".json":
            _fail("task_repair_unknown_storage", "V2 repair does not remove unknown non-TASK storage.", path=str(path))
        result[path.relative_to(root).as_posix()] = read_raw(path)
    return result


def _sources(root: Path, artifacts: dict[str, str]) -> dict[str, bytes]:
    result = _item_paths(root, artifacts["task"])
    for path in (artifacts["task"], artifacts["execution"] + "/index.json"):
        resolved = storage_path(root, path)
        if resolved.is_file():
            result[path] = read_raw(resolved)
    return result


def _request(raw: bytes, root: Path) -> dict[str, Any]:
    value = strict_keys(
        parse_json_contract(raw, source="v2 TASK repair request"), location="task_repair_v2",
        required={"schema", "stage", "requirement_id", "artifacts", "expected", "decisions", "task_index", "task_items"},
    )
    if value["schema"] != "work-task-repair-request/v2" or value["stage"] not in {"format", "complete"}:
        _fail("task_repair_schema", "Use work-task-repair-request/v2 and stage format or complete.")
    requirement = nonempty_string(value["requirement_id"], location="requirement_id")
    artifacts = _artifacts(root, requirement, value["artifacts"])
    if value["task_index"].get("requirement_id") != requirement or value["task_index"].get("artifacts") != artifacts:
        _fail("task_repair_identity", "Repair must preserve requirement and artifact routing.")
    if not isinstance(value["decisions"], list) or not value["decisions"]:
        _fail("task_repair_decisions", "V2 repair requires explicit reviewed decisions.")
    expected = value["expected"]
    if not isinstance(expected, dict) or any(not isinstance(path, str) or (fingerprint is not None and not isinstance(fingerprint, str)) for path, fingerprint in expected.items()):
        _fail("task_repair_expected", "V2 expected evidence must map paths to fingerprints or null.")
    for fingerprint in expected.values():
        if fingerprint is not None:
            sha256(fingerprint, location="expected")
    return value


def _build(request: dict[str, Any], *, root: Path, user_root: str, skill_roots=None) -> dict[str, Any]:
    artifacts = request["artifacts"]
    require_idle_writer(root, artifacts["execution"])
    require_no_spec_update(root, artifacts["execution"])
    candidate = _candidate(request, root=root, user_root=user_root, skill_roots=skill_roots)
    source = _sources(root, artifacts)
    candidate_paths = {artifacts["task"]: candidate["index_raw"]}
    directory = artifacts["task"].rsplit("/", 1)[0]
    candidate_paths.update({f"{directory}/tasks/{task_id}.json": raw for task_id, raw in candidate["item_raw"].items()})
    evidence_paths = sorted(set(source) | set(candidate_paths) | {artifacts["execution"] + "/index.json"})
    observed = {path: raw_sha256(source[path]) if path in source else None for path in evidence_paths}
    if observed != request["expected"]:
        _fail("task_repair_source_changed", "The reviewed v2 artifact set changed.")
    orphan_paths = sorted(path for path in source if "/tasks/" in path and path not in candidate_paths)
    decisions = {item.get("location") for item in request["decisions"] if isinstance(item, dict)}
    missing_orphan_decisions = [path for path in orphan_paths if f"/orphans/{Path(path).name}" not in decisions]
    if missing_orphan_decisions:
        _fail("task_repair_orphan_decision", "Orphans require explicit removal decisions and are never added automatically.", paths=missing_orphan_decisions)
    execution_path = artifacts["execution"] + "/index.json"
    if execution_path not in source:
        _fail("task_repair_execution_missing", "V2 repair requires preserved execution history and index evidence.")
    old_execution = parse_json_contract(source[execution_path], source=execution_path)
    validate_execution_index(source[execution_path], source=execution_path)
    affected = sorted(candidate["item_raw"]) if any(source.get(path) != raw for path, raw in candidate_paths.items()) else []
    repaired_execution = _execution(
        old_execution, candidate["validation"]["logical_contract"], candidate["validation"], affected,
        "TASK-REPAIR-" + raw_sha256(_render(request))[:12].upper(),
    )
    execution_raw = render_execution_index(repaired_execution)
    validate_execution_index(execution_raw, source="candidate execution index")
    candidate_paths[execution_path] = execution_raw
    files = []
    for path in sorted(set(source) | set(candidate_paths), key=lambda value: (20 if "/tasks/" in value else 30 if value == artifacts["task"] else 40, value)):
        before, after = source.get(path), candidate_paths.get(path)
        if before == after:
            continue
        row = {"phase": 20 if "/tasks/" in path else 30 if path == artifacts["task"] else 40,
               "path": path, "operation": "add" if before is None else "remove" if after is None else "replace"}
        if before is not None:
            row["before"] = encode_snapshot(before)
        if after is not None:
            row["after"] = encode_snapshot(after)
        files.append(row)
    if not files:
        _fail("task_repair_no_change", "The v2 repair candidate makes no change.")
    transaction_id = "TASK-REPAIR-" + raw_sha256(_render(request))[:12].upper()
    metadata = {
        "request": request, "artifacts": artifacts, "affected_task_ids": affected,
        "history_sha256": _history(root, artifacts["execution"]),
        "source_sha256": {path: raw_sha256(raw) for path, raw in source.items()},
        "candidate_sha256": {path: raw_sha256(raw) for path, raw in candidate_paths.items()},
    }
    transaction = {"schema": "work-spec-transaction/v2", "transaction_id": transaction_id,
                   "approval_sha256": transaction_approval_sha256(files, metadata),
                   "state": "prepared", "published_count": 0, "metadata": metadata, "files": files}
    render_spec_transaction(transaction)
    return {"transaction": transaction, "candidate": candidate, "source": source,
            "candidate_paths": candidate_paths, "affected": affected}


def _preview(request: dict[str, Any], built: dict[str, Any], diagnostics: dict[str, Any]) -> dict[str, Any]:
    transaction = built["transaction"]
    return {"schema": "work-task-repair/v2", "status": "preview", "stage": request["stage"],
            "approved_sha256": transaction["approval_sha256"], "artifacts": request["artifacts"],
            "decisions": request["decisions"], "affected_task_ids": built["affected"],
            "changed_paths": [row["path"] for row in transaction["files"]],
            "task_diagnostics": diagnostics, "file_readiness": "requires_execute_preflight"}


def prepare_v2(raw: bytes, *, project_root: Path, user_config_root: str, skill_roots=None, output_file: str | None = None) -> dict[str, Any]:
    value = strict_keys(
        parse_json_contract(raw, source="v2 TASK repair preparation"), location="task_repair_prepare_v2",
        required={"schema", "stage", "requirement_id", "artifacts", "decisions", "task_index", "task_items"},
    )
    if value["schema"] != "work-task-repair-prepare-request/v2":
        _fail("task_repair_prepare_schema", "Use work-task-repair-prepare-request/v2.")
    artifacts = _artifacts(project_root, value["requirement_id"], value["artifacts"])
    candidate_paths = {artifacts["task"]}
    directory = artifacts["task"].rsplit("/", 1)[0]
    candidate_paths.update(f"{directory}/tasks/{task_id}.json" for task_id in value["task_items"])
    source = _sources(project_root, artifacts)
    paths = sorted(set(source) | candidate_paths | {artifacts["execution"] + "/index.json"})
    prepared = {**copy.deepcopy(value), "schema": "work-task-repair-request/v2",
                "expected": {path: raw_sha256(source[path]) if path in source else None for path in paths}}
    prepared = parse_json_contract(_render(prepared), source="prepared v2 TASK repair")
    preview = repair_v2(_render(prepared), project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    if _sources(project_root, artifacts) != source:
        _fail("task_repair_source_changed", "V2 repair sources changed during preparation.")
    if output_file is not None:
        with Path(output_file).open("xb") as stream:
            stream.write(_render(prepared))
    return {"schema": "work-task-repair-prepare/v2", "request": prepared, "preview": preview, "output_file": output_file}


def repair_v2(raw: bytes, *, project_root: Path, user_config_root: str, skill_roots=None,
              operation: str = "validate", approved_sha256: str | None = None) -> dict[str, Any]:
    if operation not in {"validate", "apply", "recover"}:
        _fail("task_repair_operation", "Unknown repair operation.")
    request = _request(raw, project_root)
    artifacts = request["artifacts"]
    transaction_id = "TASK-REPAIR-" + raw_sha256(_render(request))[:12].upper()
    journal_relative = artifacts["execution"] + "/.work-task-repair-" + transaction_id + ".json"
    marker_relative = journal_relative + ".done"
    diagnostics = diagnose_task_collection(project_root, user_config_root, artifacts["task"], skill_roots=skill_roots)
    if operation == "recover":
        transaction = validate_spec_transaction(read_raw(storage_path(project_root, journal_relative)), source=journal_relative)
        if transaction["metadata"]["request"] != request:
            _fail("task_repair_recovery_changed", "Recovery requires the identical approved request.")
        built = {"transaction": transaction, "affected": transaction["metadata"]["affected_task_ids"]}
    else:
        built = _build(request, root=project_root, user_root=user_config_root, skill_roots=skill_roots)
        transaction = built["transaction"]
    result = _preview(request, built, diagnostics)
    if operation == "validate":
        return result
    sha256(approved_sha256, location="approved_sha256")
    if approved_sha256 != transaction["approval_sha256"]:
        _fail("task_repair_approval_changed", "The approved v2 repair transaction changed.")
    ignored = journal_relative if operation == "recover" else None
    require_no_spec_update(project_root, artifacts["execution"], ignored_record=ignored)
    if _history(project_root, artifacts["execution"]) != transaction["metadata"]["history_sha256"]:
        _fail("task_repair_history_changed", "Execution history changed before repair publication.")
    with state_writer(project_root, artifacts["execution"]):
        try:
            if operation == "apply":
                spec_transactions.write_journal(storage_path(project_root, journal_relative), transaction)
            published = spec_transactions.publish_journal(project_root, journal_relative, marker_relative)
        except (OSError, WorkError) as error:
            raise WorkError(ExitCode.IO_FAILURE, "task_repair_interrupted",
                            "Preserve the v2 repair transaction and obtain recovery authorization.",
                            {"recovery_required": True, "record": journal_relative}) from error
    final = diagnose_task_collection(project_root, user_config_root, artifacts["task"], skill_roots=skill_roots)
    if not final["normal_use_allowed"]:
        _fail("task_repair_post_validation", "The installed v2 repair is not valid.", task_diagnostics=final)
    result["task_diagnostics"] = final
    result["status"] = (
        "already_completed"
        if published["status"] == "already_published"
        else "recovered" if operation == "recover" else "repaired"
    )
    result["publication_status"] = published["status"]
    return result
