"""Prepare, publish, recover, and verify TASK collection revisions."""

from __future__ import annotations

import copy
from datetime import date
from pathlib import Path
from typing import Any

from pydantic import ValidationError

from ...models.plan import TOP_OPTIONAL, TOP_REQUIRED
from ...models.skill import SkillRoot
from ...services.hierarchy.fingerprint import hierarchy_selection_sha256
from ...services.hierarchy.path import build_hierarchy
from ...services.hierarchy.validation import validate_hierarchy_selection_snapshot
from ...services.instruction.catalog import (
    build_cross_mode_instruction_catalog,
    build_instruction_catalog,
)
from ...services.instruction.hierarchy import instruction_hierarchy_projection
from ...services.instruction.history import stored_selection
from ...services.instruction.source import load_instruction_sources
from ...services.instruction.root import instruction_root
from ...services.instruction.work_selection import validate_work_instruction_selection
from ...services.plan import document as plan_document
from ...services.plan.ordering import order_plan_contract
from ...services.attempt.validation import (
    build_initial_execution_index,
    derive_overall_status,
    render_execution_index,
    validate_execution_index,
)
from ...services.specification.transaction import (
    derived_transaction_id,
    completion_marker_matches,
    encode_snapshot,
    render_spec_transaction,
    require_no_spec_update,
    transaction_approval_sha256,
    validate_spec_transaction,
)
from ...models.common.validation import ContractValuePolicy
from ...models.specification.contracts import (
    SpecificationPrepareRequestContract, SpecificationUpdateRequestContract,
    SpecificationVerificationRequestContract,
    SpecificationPrepareContract, SpecificationUpdateContract, SpecificationVerificationContract,
)
from ...services.specification import transaction as spec_transactions
from ...services.specification.document_io import parse_json_contract, raw_sha256
from ...services.specification.history import execution_history_fingerprints
from ...services.specification.storage import read_raw, storage_path, write_prepared_output
from ...services.specification.semantic_edit import formalize_specification_edits
from ...services.specification.source_resolution import resolve_plan_path
from ...services.task.draft.validation import build_semantic_task_candidate, build_semantic_task_patch
from ...services.specification.writer_lock import require_idle_writer, state_writer
from ...models.common.errors import ExitCode, WorkError
from ...services.plan.validation import validate_plan_contract as validate_plan_value
from ...services.skill_catalog.catalog import snapshot_catalog_skill
from ...services.skill_selection.selection import validate_skill_roots, validate_skill_selection


nonempty_string = ContractValuePolicy.nonempty_string
sha256 = ContractValuePolicy.sha256
strict_keys = ContractValuePolicy.strict_keys


def rebuild_execution_index(
    old: dict[str, Any],
    logical: dict[str, Any],
    validation: dict[str, Any],
    affected: list[str],
    change_id: str,
) -> dict[str, Any]:
    generated = build_initial_execution_index(logical, validation)
    old_rows = {row["id"]: row for row in old["tasks"]}
    result = copy.deepcopy(generated)
    result["tasks"] = []
    for fresh in generated["tasks"]:
        row = copy.deepcopy(old_rows.get(fresh["id"], fresh))
        for field in ("skill_id", "instructions_sha256"):
            row[field] = fresh[field]
        if row["id"] in affected and row["status"] != "pending":
            if row["status"] == "cancelled":
                raise WorkError(
                    ExitCode.ARTIFACT_INTEGRITY,
                    "spec_update_cancelled_task",
                    "A cancelled TASK requires an explicit lifecycle decision.",
                )
            row["status"] = "pending_retry" if "latest_attempt" in row else "blocked"
            row["status_reason"] = {"kind": "task_change", "ref": change_id}
        result["tasks"].append(row)
    result["overall_status"] = derive_overall_status([row["status"] for row in result["tasks"]])
    return result


def _publication_followup(result: dict[str, object]) -> None:
    result["verification_request"] = SpecificationVerificationRequestContract.model_validate({
        "schema": "work-spec-verification-request/v1",
        "requirement_id": result["requirement_id"],
        "artifacts": result["artifacts"],
        "record_id": result["record_id"],
    }).to_canonical_dict()
    result["next_step"] = {"command": "task spec-verify", "input": "verification_request"}

PLAN_FIELDS = {"title", "summary", "goals", "scope", "constraints", "dependencies", "risks", "milestones", "deliverables", "acceptance_criteria", "decisions"}
INDEX_FIELDS = {"title", "summary", "decisions", "execution_defaults"}
ITEM_FIELDS = {"title", "goal", "traceability", "dependencies", "inputs", "decisions", "files", "risks", "steps", "validations", "commands", "operations"}


def _hierarchy(value: object, work_root: Path) -> dict[str, object]:
    catalog = build_cross_mode_instruction_catalog(work_root).as_dict()
    decision, selected, entries, stored = validate_hierarchy_selection_snapshot(value, catalog)
    build_hierarchy("plan", selected)
    catalog_sha256 = catalog["catalog_sha256"]
    assert isinstance(catalog_sha256, str)
    if stored != hierarchy_selection_sha256(decision, selected, entries, catalog_sha256):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "hierarchy_selection_fingerprint_mismatch",
            "The hierarchy selection fingerprint does not match its contents.",
        )
    assert isinstance(value, dict)
    return dict(value)


def _instruction_sources(work_root: Path, selected: list[str]):
    catalog = build_instruction_catalog(work_root, "plan")
    hierarchy = build_hierarchy("plan", selected)
    cross_mode = build_cross_mode_instruction_catalog(work_root) if hierarchy.selected_paths else None
    projected = instruction_hierarchy_projection(catalog, hierarchy, cross_mode)
    if projected is not None:
        resolved = build_hierarchy("plan", projected)
        hierarchy = type(hierarchy)(
            schema="work-hierarchy/v1",
            work_directory="plan",
            selected_paths=hierarchy.selected_paths,
            resolved_paths=resolved.resolved_paths,
            required_paths=resolved.required_paths,
            optional_paths=resolved.optional_paths,
        )
    return load_instruction_sources(work_root, "plan", hierarchy)


def _skill_snapshots(
    value: object,
    roots: list[SkillRoot],
) -> dict[tuple[str, str, str], dict[str, object]]:
    roots_by_identity = validate_skill_roots(roots)
    result: dict[tuple[str, str, str], dict[str, object]] = {}
    if not isinstance(value, dict) or not isinstance(value.get("skills"), list):
        return result
    for item in value["skills"]:
        if not isinstance(item, dict):
            continue
        scope, locator, source = item.get("scope"), item.get("root"), item.get("source")
        if not all(isinstance(part, str) and part.strip() for part in (scope, locator, source)):
            continue
        root = roots_by_identity.get((scope, locator))
        if root is not None:
            result[(scope, locator, source)] = snapshot_catalog_skill(root, source)
    return result


def _render_plan_contract(contract: dict[str, Any]) -> bytes:
    return plan_document.render(order_plan_contract(contract))


def _validate_plan_contract(
    raw: bytes,
    *,
    source: str,
    actual_plan_path: str,
    project_root: Path,
    skill_roots: list[SkillRoot] | None,
    allow_task_index: bool,
) -> dict[str, object]:
    contract = plan_document.parse(raw, source=source)
    strict_keys(contract, location="plan", required=TOP_REQUIRED, optional=TOP_OPTIONAL)
    work_root = plan_document.installed_work_root()
    hierarchy = _hierarchy(contract.get("hierarchy_selection"), work_root)
    selected = hierarchy["selected_paths"]
    assert isinstance(selected, list)
    current = _instruction_sources(work_root, selected)
    instruction_sha = validate_work_instruction_selection(
        contract.get("work_instruction_selection"),
        current,
        selected_paths=selected,
    ).instructions_sha256
    roots = skill_roots or []
    skill_validation = validate_skill_selection(
        contract.get("skill_selection"),
        roots=roots,
        snapshots=_skill_snapshots(contract.get("skill_selection"), roots),
    )
    skill = skill_validation["skill_selection"]
    assert isinstance(skill, dict)
    return validate_plan_value(
        raw,
        source=source,
        actual_plan_path=actual_plan_path,
        project_root=project_root,
        hierarchy_selection=hierarchy,
        instruction_fingerprint=instruction_sha,
        skill_selection_sha256=skill["selection_sha256"],
        ordered_contract=order_plan_contract(contract),
        _allow_task_index=allow_task_index,
    )


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _artifacts(plan: dict[str, Any], plan_path: str) -> dict[str, str]:
    artifacts = strict_keys(plan.get("artifacts"), location="artifacts", required={"plan", "task", "execution"})
    if artifacts["plan"] != plan_path or not str(artifacts["task"]).endswith("/index.json"):
        _fail("spec_artifact_identity", "The Plan must route this revision to a TASK collection index.")
    return artifacts


def _load(
    root: Path, user_root: str, plan_path: str, *, task_operations, skill_roots=None,
) -> dict[str, Any]:
    plan_raw = read_raw(storage_path(root, plan_path))
    plan = parse_json_contract(plan_raw, source=plan_path)
    artifacts = _artifacts(plan, plan_path)
    index_path = artifacts["task"]
    index_raw = read_raw(storage_path(root, index_path))
    index = parse_json_contract(index_raw, source=index_path)
    item_raw: dict[str, bytes] = {}
    items: dict[str, dict[str, Any]] = {}
    directory = index_path.rsplit("/", 1)[0]
    for reference in index["tasks"]:
        relative = f"{directory}/{reference['path']}"
        raw = read_raw(storage_path(root, relative))
        item_raw[reference["id"]] = raw
        items[reference["id"]] = parse_json_contract(raw, source=relative)
    validation = task_operations.validate_task_collection_contract(
        index_raw, item_raw, source=index_path, actual_index_path=index_path,
        project_root=root, user_config_root=user_root, skill_roots=skill_roots,
        _source_plan_raw=plan_raw,
    )
    execution_path = artifacts["execution"] + "/index.json"
    execution_raw = read_raw(storage_path(root, execution_path))
    validate_execution_index(execution_raw, source=execution_path)
    return {"plan": plan, "plan_raw": plan_raw, "artifacts": artifacts,
            "index": index, "index_raw": index_raw, "items": items,
            "item_raw": item_raw, "validation": validation,
            "execution": parse_json_contract(execution_raw, source=execution_path),
            "execution_raw": execution_raw, "execution_path": execution_path}


def _pointer(value: dict[str, Any], pointer: str, *, operation: str, before: object = None, after: object = None) -> None:
    if pointer == "/":
        _fail("spec_root_edit", "Whole-document edits are handled by TASK item add/remove operations.")
    if not pointer.startswith("/") or "/" in pointer[1:]:
        _fail("spec_pointer", "Preparation supports one top-level JSON Pointer per edit.", path=pointer)
    key = pointer[1:].replace("~1", "/").replace("~0", "~")
    present = key in value
    if operation == "add":
        if present:
            _fail("spec_edit_state", "An add target already exists.", path=pointer)
        value[key] = copy.deepcopy(after)
    elif operation == "replace":
        if not present or value[key] != before:
            _fail("spec_edit_state", "A replace target does not match reviewed before evidence.", path=pointer)
        value[key] = copy.deepcopy(after)
    elif operation == "remove":
        if not present or value[key] != before:
            _fail("spec_edit_state", "A remove target does not match reviewed before evidence.", path=pointer)
        del value[key]
    else:
        _fail("spec_edit_operation", "The edit operation is invalid.")


def _affected(old_items: dict[str, Any], items: dict[str, Any], *, shared: bool) -> list[str]:
    removed = set(old_items) - set(items)
    affected = set(items) if shared or removed else {task_id for task_id in items if items[task_id] != old_items.get(task_id)}
    while True:
        downstream = {task_id for task_id, item in items.items() if set(item.get("dependencies", [])) & affected}
        if downstream <= affected:
            return sorted(affected)
        affected |= downstream


def _changed_collection_fields(
    before_plan: dict[str, Any],
    after_plan: dict[str, Any],
    before_index: dict[str, Any],
    after_index: dict[str, Any],
    before_items: dict[str, dict[str, Any]],
    after_items: dict[str, dict[str, Any]],
) -> list[str]:
    """Return stable semantic paths without generated revision evidence."""
    result = {
        "/plan/" + key
        for key in before_plan.keys() | after_plan.keys()
        if key != "changes" and before_plan.get(key) != after_plan.get(key)
    }
    for key in (before_index.keys() | after_index.keys()) - {
        "tasks", "spec_id", "readiness", "changes",
    }:
        if before_index.get(key) != after_index.get(key):
            result.add("/task_index/" + key)
    for task_id in before_items.keys() | after_items.keys():
        if task_id not in before_items or task_id not in after_items:
            result.add("/task_items/" + task_id)
            continue
        before, after = before_items[task_id], after_items[task_id]
        for key in before.keys() | after.keys():
            if before.get(key) != after.get(key):
                result.add(f"/task_items/{task_id}/{key}")
    return sorted(result)


def prepare_specification(raw_request: bytes, *, project_root: Path, user_config_root: str,
                          task_operations, skill_roots=None,
                          output_file: str | None = None) -> dict[str, object]:
    request = SpecificationPrepareRequestContract.parse_json_bytes(
        raw_request, source="specification preparation",
    ).to_canonical_dict()
    plan_path = resolve_plan_path(project_root, request["requirement_id"])
    baseline = _load(
        project_root, user_config_root, plan_path,
        task_operations=task_operations, skill_roots=skill_roots,
    )
    require_no_spec_update(project_root, baseline["artifacts"]["execution"])
    require_idle_writer(project_root, baseline["artifacts"]["execution"])
    plan, index, items = (copy.deepcopy(baseline[key]) for key in ("plan", "index", "items"))
    edits = formalize_specification_edits(
        request["edits"], baseline, task_operations,
        skill_root=instruction_root(), task_candidate_builder=build_semantic_task_candidate,
        task_patch_builder=build_semantic_task_patch,
    )
    seen: set[tuple[object, ...]] = set()
    normalized: list[dict[str, Any]] = []
    plan_changed = False
    for position, edit in enumerate(edits):
        artifact, operation, pointer = edit["artifact"], edit["operation"], edit["path"]
        identity = (artifact, edit.get("task_id"), pointer)
        if identity in seen:
            _fail("spec_prepare_duplicate", "Each collection target may be edited once.")
        seen.add(identity)
        if artifact == "plan":
            if "task_id" in edit or "affected_ids" not in edit:
                _fail("spec_plan_edit", "Plan edits require affected_ids and reject task_id.")
            if pointer[1:] not in PLAN_FIELDS:
                _fail("spec_prepare_field", "This Plan field is protected.", path=pointer)
            _pointer(plan, pointer, operation=operation, before=edit.get("before"), after=edit.get("after"))
            plan_changed = True
        elif artifact == "task_index":
            if "task_id" in edit:
                _fail("spec_index_edit", "TASK index edits reject task_id.")
            if pointer[1:] not in INDEX_FIELDS:
                _fail("spec_prepare_field", "This TASK index field is protected.", path=pointer)
            _pointer(index, pointer, operation=operation, before=edit.get("before"), after=edit.get("after"))
        elif artifact == "task_item":
            task_id = nonempty_string(edit.get("task_id"), location=f"edits[{position}].task_id")
            if pointer == "/" and operation == "add":
                if task_id in items or not isinstance(edit.get("after"), dict):
                    _fail("spec_item_add", "A semantic TASK addition must produce one new TASK item.")
                items[task_id] = copy.deepcopy(edit["after"])
            elif pointer == "/" and operation == "remove":
                if task_id not in items or edit.get("before") != items[task_id]:
                    _fail("spec_item_remove", "Removed TASK item evidence does not match.")
                del items[task_id]
            else:
                if task_id not in items:
                    _fail("spec_prepare_task_id", "Unknown TASK ID.")
                if pointer[1:] not in ITEM_FIELDS:
                    _fail("spec_prepare_field", "This TASK item field is protected.", path=pointer)
                _pointer(items[task_id], pointer, operation=operation, before=edit.get("before"), after=edit.get("after"))
        else:
            _fail("spec_prepare_artifact", "Use plan, task_index or task_item.")
        if artifact != "plan":
            normalized.append({key: copy.deepcopy(edit[key]) for key in ("artifact", "task_id", "operation", "path", "before", "after") if key in edit})
    if any(edit["artifact"] == "task_item" and edit["path"] == "/" for edit in edits):
        index["instruction_selection"] = task_operations.build_task_document_instruction_selection(
            [items[task_id]["instruction_selection"] for task_id in sorted(items)],
            skill_root=instruction_root(),
        )
    plan_raw = _render_plan_contract(plan)
    plan_validation = _validate_plan_contract(
        plan_raw,
        source="candidate Plan",
        actual_plan_path=plan_path,
        project_root=project_root,
        skill_roots=skill_roots,
        allow_task_index=True,
    )
    next_number = int(baseline["index"]["spec_id"].rsplit("-", 1)[1]) + 1
    index["spec_id"] = f"TASK-SPEC-{next_number:03d}"
    index["source_plan"]["canonical_sha256"] = plan_validation["plan_sha256"]
    index["readiness"]["spec_id"] = index["spec_id"]
    item_raw = {task_id: task_operations.render_task_item_contract(item) for task_id, item in items.items()}
    references = []
    directory = baseline["artifacts"]["task"].rsplit("/", 1)[0]
    for task_id in sorted(items):
        validation = task_operations.validate_task_item_contract(item_raw[task_id], source=task_id, expected_task_id=task_id)
        references.append({"id": task_id, "path": f"tasks/{task_id}.json", "canonical_sha256": validation["task_item_sha256"]})
    index["tasks"] = references
    shared = plan_changed or any(edit["artifact"] == "task_index" for edit in normalized)
    affected = _affected(baseline["items"], items, shared=shared)
    change_id = f"TASK-CHANGE-{max((int(row['id'].rsplit('-', 1)[1]) for row in baseline['index'].get('changes', [])), default=0) + 1:03d}"
    if not normalized:
        normalized = [{"artifact": "task_index", "operation": "replace", "path": "/source_plan", "before": baseline["index"]["source_plan"], "after": index["source_plan"]}]
    index["changes"] = list(baseline["index"].get("changes", [])) + [{"id": change_id, "spec_id": index["spec_id"], "date": date.today().isoformat(), "reason": request["reason"], "affected_ids": affected, "edits": normalized}]
    index_raw = task_operations.render_task_index_contract(index)
    validation = task_operations.validate_task_collection_contract(index_raw, item_raw, source=baseline["artifacts"]["task"], actual_index_path=baseline["artifacts"]["task"],
                                                   project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
                                                   validate_file_state=False, _source_plan_raw=plan_raw)
    execution = rebuild_execution_index(baseline["execution"], validation["collection_contract"], validation, affected, change_id)
    execution_raw = render_execution_index(execution)
    validate_execution_index(execution_raw, source="candidate execution index")
    prepared = {"schema": "work-spec-update-request/v1", "reason": request["reason"],
                "expected": {"plan_sha256": raw_sha256(baseline["plan_raw"]), "task_index_sha256": raw_sha256(baseline["index_raw"]),
                             "execution_index_sha256": raw_sha256(baseline["execution_raw"]),
                             "task_item_sha256": {key: raw_sha256(value) for key, value in baseline["item_raw"].items()}},
                "plan": plan, "task_index": index, "task_items": items}
    preview = _validate_collection(
        prepared, baseline=baseline, project_root=project_root,
        user_config_root=user_config_root, task_operations=task_operations,
        skill_roots=skill_roots,
    )
    prepared = preview["transaction"]["metadata"]["request"]
    if output_file is not None:
        write_prepared_output(output_file, prepared)
    return SpecificationPrepareContract.model_validate({"schema": "work-spec-prepare/v1", "request": prepared, "preview": preview, "output_file": output_file,
            "transport": {"request_field": "request", "request_schema": prepared["schema"], "output_file": output_file},
            "next_step": {"command": "task spec-validate", "input": "request"}}).to_canonical_dict()


def _candidate_sources(request: dict[str, Any], baseline: dict[str, Any], *, project_root: Path,
                       user_config_root: str, task_operations, skill_roots=None):
    plan_raw = _render_plan_contract(request["plan"])
    index_raw = task_operations.render_task_index_contract(request["task_index"])
    item_raw = {key: task_operations.render_task_item_contract(value) for key, value in request["task_items"].items()}
    validation = task_operations.validate_task_collection_contract(index_raw, item_raw, source=baseline["artifacts"]["task"], actual_index_path=baseline["artifacts"]["task"],
                                                   project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
                                                   validate_file_state=False, _source_plan_raw=plan_raw)
    affected = request["task_index"]["changes"][-1]["affected_ids"]
    execution = rebuild_execution_index(baseline["execution"], validation["collection_contract"], validation, affected, request["task_index"]["changes"][-1]["id"])
    return plan_raw, index_raw, item_raw, render_execution_index(execution), validation, affected


def _validate_collection(request: dict[str, Any], *, baseline: dict[str, Any], project_root: Path,
                         user_config_root: str, task_operations, skill_roots=None) -> dict[str, object]:
    try:
        request = SpecificationUpdateRequestContract.model_validate(request).to_canonical_dict()
    except ValidationError as error:
        raise SpecificationUpdateRequestContract._work_error(error) from error
    expected = request["expected"]
    observed = {"plan_sha256": raw_sha256(baseline["plan_raw"]), "task_index_sha256": raw_sha256(baseline["index_raw"]),
                "execution_index_sha256": raw_sha256(baseline["execution_raw"]), "task_item_sha256": {key: raw_sha256(value) for key, value in baseline["item_raw"].items()}}
    if expected != observed:
        _fail("spec_update_source_changed", "The reviewed source fingerprints changed.")
    if "lock" in baseline["execution"]:
        _fail("spec_update_lock_present", "The execution index already contains a lock.")
    plan_raw, index_raw, item_raw, execution_raw, validation, affected = _candidate_sources(
        request, baseline, project_root=project_root, user_config_root=user_config_root,
        task_operations=task_operations, skill_roots=skill_roots,
    )
    candidate = {baseline["artifacts"]["plan"]: plan_raw, baseline["artifacts"]["task"]: index_raw,
                 baseline["execution_path"]: execution_raw}
    directory = baseline["artifacts"]["task"].rsplit("/", 1)[0]
    candidate.update({f"{directory}/tasks/{key}.json": value for key, value in item_raw.items()})
    source = {baseline["artifacts"]["plan"]: baseline["plan_raw"], baseline["artifacts"]["task"]: baseline["index_raw"],
              baseline["execution_path"]: baseline["execution_raw"]}
    source.update({f"{directory}/tasks/{key}.json": value for key, value in baseline["item_raw"].items()})
    files = []
    for path in sorted(source.keys() | candidate.keys(), key=lambda value: ((20 if "/tasks/" in value else 10 if value == baseline["artifacts"]["plan"] else 30 if value == baseline["artifacts"]["task"] else 40), value)):
        before, after = source.get(path), candidate.get(path)
        if before == after:
            continue
        phase = 20 if "/tasks/" in path else 10 if path == baseline["artifacts"]["plan"] else 30 if path == baseline["artifacts"]["task"] else 40
        row = {"phase": phase, "path": path, "operation": "add" if before is None else "remove" if after is None else "replace"}
        if before is not None: row["before"] = encode_snapshot(before)
        if after is not None: row["after"] = encode_snapshot(after)
        files.append(row)
    metadata = {"request": request, "artifacts": baseline["artifacts"], "affected_task_ids": affected,
                "history_sha256": execution_history_fingerprints(project_root, baseline["artifacts"]["execution"]),
                "source_sha256": {key: raw_sha256(value) for key, value in source.items()},
                "candidate_sha256": {key: raw_sha256(value) for key, value in candidate.items()}}
    approval = transaction_approval_sha256(files, metadata)
    transaction_id = derived_transaction_id("UPDATE", approval)
    journal = {"schema": "work-spec-transaction/v1", "transaction_id": transaction_id,
               "approval_sha256": approval, "state": "prepared", "published_count": 0,
               "metadata": metadata, "files": files}
    render_spec_transaction(journal)
    changed_fields = _changed_collection_fields(
        baseline["plan"], request["plan"], baseline["index"], request["task_index"],
        baseline["items"], request["task_items"],
    )
    return SpecificationUpdateContract.model_validate({"schema": "work-spec-update/v1", "status": "valid", "requirement_id": request["plan"]["requirement_id"],
            "record_id": transaction_id, "approved_sha256": journal["approval_sha256"], "affected_task_ids": affected,
            "changed_fields": changed_fields,
            "artifacts": baseline["artifacts"], "candidate": {"plan": request["plan"], "task_index": request["task_index"], "task_items": request["task_items"]},
            "transaction": journal, "file_readiness": "requires_execute_preflight"}).to_canonical_dict()


def update_specification(raw_request: bytes, *, project_root: Path, user_config_root: str,
                         task_operations, skill_roots=None, operation="validate",
                         approved_sha256=None) -> dict[str, object]:
    request = SpecificationUpdateRequestContract.parse_json_bytes(
        raw_request, source="specification update",
    ).to_canonical_dict()
    plan = request["plan"]
    artifacts = _artifacts(plan, plan["artifacts"]["plan"])
    if operation == "recover":
        sha256(approved_sha256, location="approved_sha256")
        transaction_id = derived_transaction_id("UPDATE", approved_sha256)
        journal_relative = artifacts["execution"] + "/.work-spec-update-" + transaction_id + ".json"
    if operation == "recover":
        journal = validate_spec_transaction(read_raw(storage_path(project_root, journal_relative)), source=journal_relative)
        if journal["metadata"]["request"] != request: _fail("spec_update_recovery_request", "Recovery requires the identical request.")
        result = {"schema": "work-spec-update/v1", "status": "valid", "requirement_id": plan["requirement_id"], "record_id": journal["transaction_id"],
                  "approved_sha256": journal["approval_sha256"], "affected_task_ids": journal["metadata"]["affected_task_ids"], "artifacts": artifacts}
    else:
        require_no_spec_update(project_root, artifacts["execution"])
        baseline = _load(
            project_root, user_config_root, artifacts["plan"],
            task_operations=task_operations, skill_roots=skill_roots,
        )
        result = _validate_collection(
            request, baseline=baseline, project_root=project_root,
            user_config_root=user_config_root, task_operations=task_operations,
            skill_roots=skill_roots,
        )
        journal = result["transaction"]
        journal_relative = artifacts["execution"] + "/.work-spec-update-" + journal["transaction_id"] + ".json"
    if operation == "validate":
        result["next_step"] = {"command": "task spec-update", "input": "same_request", "approved_sha256": result["approved_sha256"]}
        return SpecificationUpdateContract.model_validate(result).to_canonical_dict()
    sha256(approved_sha256, location="approved_sha256")
    if approved_sha256 != result["approved_sha256"]: _fail("spec_update_approval_changed", "The approved transaction changed.")
    if execution_history_fingerprints(project_root, artifacts["execution"]) != journal["metadata"]["history_sha256"]: _fail("spec_update_history_changed", "Execution history changed before publication.")
    marker_relative = journal_relative + ".done"
    with state_writer(project_root, artifacts["execution"]):
        if execution_history_fingerprints(project_root, artifacts["execution"]) != journal["metadata"]["history_sha256"]:
            _fail("spec_update_history_changed", "Execution history changed before exclusive publication.")
        try:
            if operation == "apply":
                spec_transactions.write_journal(storage_path(project_root, journal_relative), journal)
            published = spec_transactions.publish_journal(project_root, journal_relative, marker_relative)
        except (OSError, WorkError) as error:
            raise WorkError(
                ExitCode.IO_FAILURE, "spec_update_interrupted",
                "Preserve the specification transaction and obtain recovery authorization.",
                {"recovery_required": True, "record": journal_relative},
            ) from error
    installed = _load(
        project_root, user_config_root, artifacts["plan"],
        task_operations=task_operations, skill_roots=skill_roots,
    )
    for path, expected_sha in journal["metadata"]["candidate_sha256"].items():
        if raw_sha256(read_raw(storage_path(project_root, path))) != expected_sha:
            _fail("spec_update_post_write", "An installed collection artifact differs from approval.", path=path)
    result.pop("transaction", None)
    result["status"] = "recovered" if operation == "recover" else "updated"
    result["publication_status"] = published["status"]
    _publication_followup(result)
    return SpecificationUpdateContract.model_validate(result).to_canonical_dict()


def verify_specification(raw_request: bytes, *, project_root: Path, user_config_root: str,
                         task_operations, skill_roots=None) -> dict[str, object]:
    request = SpecificationVerificationRequestContract.parse_json_bytes(
        raw_request, source="specification verification",
    ).to_canonical_dict()
    artifacts = request["artifacts"]
    journal_relative = artifacts["execution"] + "/.work-spec-update-" + request["record_id"] + ".json"
    raw = read_raw(storage_path(project_root, journal_relative))
    journal = validate_spec_transaction(raw, source=journal_relative)
    marker = read_raw(storage_path(project_root, journal_relative + ".done"))
    if not completion_marker_matches(raw, marker): _fail("spec_verify_completion_mismatch", "The completion marker does not match.")
    for path, expected in journal["metadata"]["candidate_sha256"].items():
        if raw_sha256(read_raw(storage_path(project_root, path))) != expected: _fail("spec_verify_state_changed", "A candidate file changed.", path=path)
    validation = _load(
        project_root, user_config_root, artifacts["plan"],
        task_operations=task_operations, skill_roots=skill_roots,
    )["validation"]
    return SpecificationVerificationContract.model_validate({"schema": "work-spec-verification/v1", "status": "verified", "verified": True,
            "record_id": request["record_id"], "requirement_id": request["requirement_id"], "artifacts": artifacts,
            "task_collection_sha256": validation["task_collection_sha256"], "journal_sha256": raw_sha256(raw),
            "verification_scope": "exact_specification_result",
            "execution_authorized": False, "next_step": "normal_execute_preflight"}).to_canonical_dict()
