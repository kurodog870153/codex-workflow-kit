"""Explicitly reviewed repair of TASK collections and execution bindings."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

from ...services.attempt.validation import render_execution_index, validate_execution_index
from ...services.specification.transaction import encode_snapshot, render_spec_transaction, transaction_approval_sha256, validate_spec_transaction
from .collection import validate_task_collection_contract
from .index import render_task_index_contract
from .item import render_task_item_contract, validate_task_item_contract
from ...models.task_collection.repair import (
    TaskRepairContract,
    TaskRepairPrepareContract,
    TaskRepairPrepareRequestContract,
    TaskRepairRequestContract,
)
from ...models.common.validation import ContractValuePolicy
from ...services.specification.transaction import publish_journal, write_journal
from ...models.common.errors import ExitCode, WorkError
from ...services.task.repair_fingerprint import (
    fingerprint_task_repair_contents,
    fingerprint_task_repair_evidence,
    task_repair_transaction_id,
)
from ...services.task.storage import read_raw
from ...services.task.document import parse_json_contract, render_json_contract
from ...services.task.storage import resolve_project_relative_path, validate_task_collection_index_path
from ...services.specification.storage import storage_path
from ...services.specification.transaction import require_no_spec_update
from ...services.specification.writer_lock import require_idle_writer, state_writer
from ...services.task.draft.validation import build_semantic_task_candidate, build_semantic_task_patch
from ...services.specification.semantic_edit import TASK_GROUPS, _index_decisions, _object, _task_positions, _traceability
from ...services.specification.source_resolution import resolve_repair_artifacts
from ...services.instruction.root import instruction_root


nonempty_string = ContractValuePolicy.nonempty_string
sha256 = ContractValuePolicy.sha256
strict_keys = ContractValuePolicy.strict_keys


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
        _fail("task_repair_candidate", "TASK collection repair requires an explicit complete index and TASK item map.")
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
            _fail("task_repair_unknown_storage", "TASK collection repair does not remove unknown non-TASK storage.", path=str(path))
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
    value = TaskRepairRequestContract.parse_json_bytes(
        raw, source="TASK collection repair request",
    ).to_canonical_dict()
    requirement = value["requirement_id"]
    artifacts = _artifacts(root, requirement, value["artifacts"])
    if value["task_index"].get("requirement_id") != requirement or value["task_index"].get("artifacts") != artifacts:
        _fail("task_repair_identity", "Repair must preserve requirement and artifact routing.")
    return value


def _build(
    request: dict[str, Any],
    *,
    root: Path,
    user_root: str,
    execution_history_fingerprints,
    rebuild_execution_index,
    skill_roots=None,
) -> dict[str, Any]:
    artifacts = request["artifacts"]
    require_idle_writer(root, artifacts["execution"])
    require_no_spec_update(root, artifacts["execution"])
    candidate = _candidate(request, root=root, user_root=user_root, skill_roots=skill_roots)
    source = _sources(root, artifacts)
    candidate_paths = {artifacts["task"]: candidate["index_raw"]}
    directory = artifacts["task"].rsplit("/", 1)[0]
    candidate_paths.update({f"{directory}/tasks/{task_id}.json": raw for task_id, raw in candidate["item_raw"].items()})
    evidence_paths = sorted(set(source) | set(candidate_paths) | {artifacts["execution"] + "/index.json"})
    observed = fingerprint_task_repair_evidence(source, evidence_paths)
    if observed != request["expected"]:
        _fail("task_repair_source_changed", "The reviewed artifact set changed.")
    orphan_paths = sorted(path for path in source if "/tasks/" in path and path not in candidate_paths)
    decisions = {item.get("location") for item in request["decisions"] if isinstance(item, dict)}
    missing_orphan_decisions = [path for path in orphan_paths if f"/orphans/{Path(path).name}" not in decisions]
    if missing_orphan_decisions:
        _fail("task_repair_orphan_decision", "Orphans require explicit removal decisions and are never added automatically.", paths=missing_orphan_decisions)
    execution_path = artifacts["execution"] + "/index.json"
    if execution_path not in source:
        _fail("task_repair_execution_missing", "TASK collection repair requires preserved execution history and index evidence.")
    old_execution = parse_json_contract(source[execution_path], source=execution_path)
    validate_execution_index(source[execution_path], source=execution_path)
    affected = sorted(candidate["item_raw"]) if any(source.get(path) != raw for path, raw in candidate_paths.items()) else []
    repaired_execution = rebuild_execution_index(
        old_execution, candidate["validation"]["collection_contract"], candidate["validation"], affected,
        task_repair_transaction_id(_render(request)),
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
        _fail("task_repair_no_change", "The repair candidate makes no change.")
    transaction_id = task_repair_transaction_id(_render(request))
    metadata = {
        "request": request, "artifacts": artifacts, "affected_task_ids": affected,
        "history_sha256": execution_history_fingerprints(root, artifacts["execution"]),
        "source_sha256": fingerprint_task_repair_contents(source),
        "candidate_sha256": fingerprint_task_repair_contents(candidate_paths),
    }
    transaction = {"schema": "work-spec-transaction/v1", "transaction_id": transaction_id,
                   "approval_sha256": transaction_approval_sha256(files, metadata),
                   "state": "prepared", "published_count": 0, "metadata": metadata, "files": files}
    render_spec_transaction(transaction)
    return {"transaction": transaction, "candidate": candidate, "source": source,
            "candidate_paths": candidate_paths, "affected": affected}


def _preview(request: dict[str, Any], built: dict[str, Any], diagnostics: dict[str, Any]) -> dict[str, Any]:
    transaction = built["transaction"]
    return TaskRepairContract.model_validate({"schema": "work-task-repair/v1", "status": "preview", "stage": request["stage"],
            "approved_sha256": transaction["approval_sha256"], "artifacts": request["artifacts"],
            "decisions": request["decisions"], "affected_task_ids": built["affected"],
            "changed_paths": [row["path"] for row in transaction["files"]],
            "task_diagnostics": diagnostics, "file_readiness": "requires_execute_preflight"}).to_canonical_dict()


def repair_task(raw: bytes, *, project_root: Path, user_config_root: str, skill_roots=None,
              operation: str = "validate", approved_sha256: str | None = None,
              execution_history_fingerprints, rebuild_execution_index,
              diagnose_task_collection) -> dict[str, Any]:
    if operation not in {"validate", "apply", "recover"}:
        _fail("task_repair_operation", "Unknown repair operation.")
    request = _request(raw, project_root)
    artifacts = request["artifacts"]
    transaction_id = task_repair_transaction_id(_render(request))
    journal_relative = artifacts["execution"] + "/.work-task-repair-" + transaction_id + ".json"
    marker_relative = journal_relative + ".done"
    diagnostics = diagnose_task_collection(project_root, user_config_root, artifacts["task"], skill_roots=skill_roots)
    if operation == "recover":
        transaction = validate_spec_transaction(read_raw(storage_path(project_root, journal_relative)), source=journal_relative)
        if transaction["metadata"]["request"] != request:
            _fail("task_repair_recovery_changed", "Recovery requires the identical approved request.")
        built = {"transaction": transaction, "affected": transaction["metadata"]["affected_task_ids"]}
    else:
        built = _build(
            request,
            root=project_root,
            user_root=user_config_root,
            skill_roots=skill_roots,
            execution_history_fingerprints=execution_history_fingerprints,
            rebuild_execution_index=rebuild_execution_index,
        )
        transaction = built["transaction"]
    result = _preview(request, built, diagnostics)
    if operation == "validate":
        return TaskRepairContract.model_validate(result).to_canonical_dict()
    sha256(approved_sha256, location="approved_sha256")
    if approved_sha256 != transaction["approval_sha256"]:
        _fail("task_repair_approval_changed", "The approved repair transaction changed.")
    ignored = journal_relative if operation == "recover" else None
    require_no_spec_update(project_root, artifacts["execution"], ignored_record=ignored)
    if execution_history_fingerprints(project_root, artifacts["execution"]) != transaction["metadata"]["history_sha256"]:
        _fail("task_repair_history_changed", "Execution history changed before repair publication.")
    with state_writer(project_root, artifacts["execution"]):
        try:
            if operation == "apply":
                write_journal(storage_path(project_root, journal_relative), transaction)
            published = publish_journal(project_root, journal_relative, marker_relative)
        except (OSError, WorkError) as error:
            raise WorkError(ExitCode.IO_FAILURE, "task_repair_interrupted",
                            "Preserve the repair transaction and obtain recovery authorization.",
                            {"recovery_required": True, "record": journal_relative}) from error
    final = diagnose_task_collection(project_root, user_config_root, artifacts["task"], skill_roots=skill_roots)
    if not final["normal_use_allowed"]:
        _fail("task_repair_post_validation", "The installed repair is not valid.", task_diagnostics=final)
    result["task_diagnostics"] = final
    result["status"] = (
        "already_completed"
        if published["status"] == "already_published"
        else "recovered" if operation == "recover" else "repaired"
    )
    result["publication_status"] = published["status"]
    return TaskRepairContract.model_validate(result).to_canonical_dict()


def prepare_task_repair(
    raw: bytes,
    *,
    project_root: Path,
    user_config_root: str,
    skill_roots=None,
    output_file: str | None = None,
    execution_history_fingerprints,
    rebuild_execution_index,
    diagnose_task_collection,
    instruction_operations,
) -> dict[str, Any]:
    value = TaskRepairPrepareRequestContract.parse_json_bytes(
        raw, source="TASK collection repair preparation",
    ).to_canonical_dict()
    artifacts = _artifacts(project_root, value["requirement_id"],
                           resolve_repair_artifacts(project_root, value["requirement_id"]))
    source = _sources(project_root, artifacts)
    index_raw = source.get(artifacts["task"])
    if index_raw is None:
        _fail("task_repair_ambiguous_source", "A missing TASK index cannot be reconstructed without a confirmed source.")
    index = parse_json_contract(index_raw, source=artifacts["task"])
    if not isinstance(index, dict) or index.get("requirement_id") != value["requirement_id"] or index.get("artifacts") != artifacts:
        _fail("task_repair_identity", "Repair must preserve requirement and artifact routing.")
    items: dict[str, dict[str, Any]] = {}
    directory = artifacts["task"].rsplit("/", 1)[0]
    references = index.get("tasks")
    if not isinstance(references, list) or not references:
        _fail("task_repair_ambiguous_source", "A TASK index with identified item references is required.")
    replacement = value.get("missing_task")
    used_replacement = False
    plan = parse_json_contract(read_raw(storage_path(project_root, artifacts["plan"])), source=artifacts["plan"])
    if not isinstance(plan, dict) or plan.get("requirement_id") != value["requirement_id"] or plan.get("artifacts") != artifacts:
        _fail("task_repair_ambiguous_source", "The source Plan does not establish the TASK identity and routing.")
    for position, reference in enumerate(references, 1):
        if not isinstance(reference, dict) or not isinstance(reference.get("id"), str) or reference.get("path") != f"tasks/{reference['id']}.json":
            _fail("task_repair_ambiguous_source", "TASK item references must have unambiguous IDs and paths.")
        task_id = reference["id"]
        path = f"{directory}/{reference['path']}"
        if replacement is not None and replacement["task_position"] == position:
            if path in source:
                _fail("task_repair_ambiguous_source", "A semantic missing TASK can only fill an absent item.", path=path)
            dependencies = [references[item - 1]["id"] for item in replacement["dependency_positions"] if type(item) is int and 1 <= item <= len(references)]
            if len(dependencies) != len(replacement["dependency_positions"]) or len(dependencies) != len(set(dependencies)) or task_id in dependencies:
                _fail("task_repair_ambiguous_source", "Missing TASK dependency positions are invalid.")
            instruction = instruction_operations.build_instruction_selection(skill_root=instruction_root(), mode="task", selected_paths=replacement["selected_paths"], reference_names=replacement["references"])
            acceptance_ids = [item["id"] for item in plan["acceptance_criteria"]]
            semantic = build_semantic_task_candidate(replacement["candidate"], acceptance_ids=acceptance_ids, dependency_ids=dependencies)
            items[task_id] = {
                "schema": "work-task-item/v1", "id": task_id, "title": replacement["title"],
                "goal": replacement["goal"], "skill_id": replacement["skill_id"],
                "instruction_selection": instruction,
                "traceability": {"goal_ids": [item["id"] for item in plan["goals"]],
                                 "deliverable_ids": [item["id"] for item in plan["deliverables"]],
                                 "acceptance_ids": acceptance_ids},
                **({"dependencies": dependencies} if dependencies else {}), **semantic,
            }
            used_replacement = True
        elif path in source:
            item = parse_json_contract(source[path], source=path)
            if not isinstance(item, dict) or item.get("id") != task_id:
                _fail("task_repair_ambiguous_source", "A source TASK item has a conflicting identity.", path=path)
            items[task_id] = item
        else:
            _fail("task_repair_ambiguous_source", "A missing TASK item requires a confirmed semantic TASK decision.", path=path)
    if replacement is not None and not used_replacement:
        _fail("task_repair_ambiguous_source", "The missing TASK position does not match an index reference.")
    selections = [items[reference["id"]]["instruction_selection"] for reference in references]
    if instruction_operations.build_task_document_instruction_selection(selections, skill_root=instruction_root()) != index["instruction_selection"]:
        _fail("task_repair_ambiguous_source", "The reconstructed TASK selection does not match the source index.")
    seen: set[tuple[str | None, str]] = set()
    protected_index = {"schema", "requirement_id", "spec_id", "artifacts", "tasks", "source_plan", "instruction_selection", "readiness"}
    protected_item = {"schema", "id", "source", "skill_id", "instruction_selection"}
    nested_changes: dict[str, dict[str, Any]] = {}
    for edit in value.get("edits") or []:
        task_id, field = edit.get("task_id"), edit["field"]
        if task_id is not None and field in TASK_GROUPS and not edit.get("remove"):
            nested_changes.setdefault(task_id, {})[field] = edit["semantic_after"]
    nested_values: dict[str, dict[str, Any]] = {}
    task_ids = [reference["id"] for reference in references]
    for task_id, replacements in nested_changes.items():
        if task_id not in items:
            _fail("task_repair_ambiguous_source", "An edit targets an unknown TASK item.", task_id=task_id)
        current = items[task_id]
        for group in TASK_GROUPS:
            rows = current.get(group) or []
            if not isinstance(rows, list) or any(not isinstance(row, dict) or not isinstance(row.get("id"), str) for row in rows):
                _fail("task_repair_ambiguous_source", "Nested source records need identifiable formal IDs.", field=group)
        dependency_edit = next((edit for edit in value.get("edits") or [] if edit.get("task_id") == task_id and edit["field"] == "dependencies"), None)
        dependency_ids = (_task_positions(dependency_edit["semantic_after"], task_ids, location="dependency_positions")
                          if dependency_edit is not None and not dependency_edit.get("remove") else current.get("dependencies") or [])
        if any(dep_id not in items for dep_id in dependency_ids):
            _fail("task_repair_ambiguous_source", "A dependency TASK source is missing.")
        dependency_files = {dep_id: {f"existing-{position}": row["id"] for position, row in enumerate(items[dep_id].get("files") or [], 1)}
                            for dep_id in dependency_ids}
        nested_values[task_id] = build_semantic_task_patch(
            current, replacements, acceptance_ids=[row["id"] for row in plan["acceptance_criteria"]],
            dependency_ids=dependency_ids, dependency_files=dependency_files,
        )
    for edit in value.get("edits") or []:
        task_id, field = edit.get("task_id"), edit["field"]
        identity = (task_id, field)
        if identity in seen:
            _fail("task_repair_duplicate_edit", "Repair edits must target each field only once.")
        seen.add(identity)
        if task_id is not None and task_id not in items:
            _fail("task_repair_ambiguous_source", "An edit targets an unknown TASK item.", task_id=task_id)
        if field in (protected_item if task_id is not None else protected_index):
            _fail("task_repair_protected_field", "Formal identity and source bindings are not semantic repair fields.", field=field)
        target = items[task_id] if task_id is not None else index
        present = field in target
        if edit.get("remove"):
            if not present:
                _fail("task_repair_edit_state", "A remove target does not exist in source.")
            del target[field]
        else:
            if "after" in edit:
                after = edit["after"]
            elif task_id is not None and field in TASK_GROUPS:
                after = nested_values[task_id][field]
            elif task_id is not None and field == "traceability":
                after = _traceability(edit["semantic_after"], plan)
            elif task_id is not None and field == "dependencies":
                after = _task_positions(edit["semantic_after"], task_ids, location="dependency_positions")
            elif field == "decisions":
                after = _index_decisions(edit["semantic_after"], index)
            elif field == "execution_defaults":
                after = _object(edit["semantic_after"], required={"working_directory", "os", "shell"}, location="execution_defaults")
            else:
                _fail("task_repair_protected_field", "This repair field has no semantic builder.", field=field)
            target[field] = copy.deepcopy(after)
    for reference in references:
        task_id = reference["id"]
        item_raw = render_task_item_contract(items[task_id])
        validation = validate_task_item_contract(item_raw, source=task_id, expected_task_id=task_id)
        reference["canonical_sha256"] = validation["task_item_sha256"]
    candidate_paths = {artifacts["task"]}
    candidate_paths.update(
        f"{directory}/tasks/{task_id}.json" for task_id in items
    )
    paths = sorted(
        set(source) | candidate_paths | {artifacts["execution"] + "/index.json"}
    )
    prepared = {
        "schema": "work-task-repair-request/v1",
        "stage": value["stage"], "requirement_id": value["requirement_id"],
        "artifacts": artifacts, "decisions": value["decisions"],
        "task_index": index, "task_items": items,
        "expected": fingerprint_task_repair_evidence(source, paths),
    }
    prepared = parse_json_contract(
        _render(prepared), source="prepared TASK collection repair"
    )
    preview = repair_task(
        _render(prepared),
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
        execution_history_fingerprints=execution_history_fingerprints,
        rebuild_execution_index=rebuild_execution_index,
        diagnose_task_collection=diagnose_task_collection,
    )
    if _sources(project_root, artifacts) != source:
        _fail(
            "task_repair_source_changed",
            "TASK collection repair sources changed during preparation.",
        )
    if output_file is not None:
        with Path(output_file).open("xb") as stream:
            stream.write(_render(prepared))
    return TaskRepairPrepareContract.model_validate(
        {
            "schema": "work-task-repair-prepare/v1",
            "request": prepared,
            "preview": preview,
            "output_file": output_file,
        }
    ).to_canonical_dict()
