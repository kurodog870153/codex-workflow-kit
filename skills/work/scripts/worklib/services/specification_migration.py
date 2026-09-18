"""Read-only diagnostics for an AI-produced specification migration candidate set."""
from __future__ import annotations

import copy
import difflib
from pathlib import Path
from typing import Any, Callable

from ..contracts.execution_index import validate_execution_index
from ..contracts.specification_migration_models import (
    SpecificationMigrationPreviewContract, SpecificationMigrationPreviewRequestContract,
    SpecificationMigrationPublicationContract,
)
from ..contracts.spec_transaction import encode_snapshot, transaction_approval_sha256, validate_spec_transaction
from ..foundation import spec_transactions
from ..contracts.task_collection import validate_task_collection_contract
from ..models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_json_sha256, raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..foundation.spec_update import require_no_spec_update, storage_path
from ..infrastructure.writer_lock import require_idle_writer, state_writer
from .specification import execution_history_fingerprints
from .plan_validation import validate_plan_contract


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _check(name: str, operation: Callable[[], object]) -> tuple[dict[str, object], object | None]:
    try:
        value = operation()
    except WorkError as error:
        return {"name": name, "status": "failed", "code": error.code, "message": error.message}, None
    return {"name": name, "status": "passed"}, value


def _diff(path: str, before: bytes | None, after: bytes | None) -> dict[str, str]:
    operation = "add" if before is None else "remove" if after is None else "replace"
    def lines(raw: bytes | None) -> list[str]:
        if raw is None:
            return []
        try:
            return raw.decode("utf-8").splitlines(keepends=True)
        except UnicodeDecodeError:
            return [f"<binary sha256={raw_sha256(raw)}>\n"]
    rendered = "".join(difflib.unified_diff(
        lines(before), lines(after), fromfile=path + ":source", tofile=path + ":candidate",
    ))
    return {"path": path, "operation": operation, "unified_diff": rendered}


def preview_specification_migration(
    raw_request: bytes,
    *,
    project_root: Path,
    user_config_root: str,
    skill_roots=None,
) -> dict[str, object]:
    request = SpecificationMigrationPreviewRequestContract.parse_json_bytes(
        raw_request, source="specification migration preview",
    ).to_canonical_dict()
    sources: dict[str, bytes] = {}
    for evidence in request["sources"]:
        path, resolved = resolve_project_relative_path(project_root, evidence["path"], field="source.path")
        if path in sources:
            _fail("migration_source_duplicate", "Migration source paths must be unique.", path=path)
        raw = read_raw(resolved)
        if raw_sha256(raw) != evidence["raw_sha256"]:
            _fail("migration_source_changed", "Migration source bytes differ from reviewed evidence.", path=path)
        sources[path] = raw

    candidates: dict[str, dict[str, Any]] = {}
    by_kind: dict[str, list[dict[str, Any]]] = {}
    for candidate in request["candidates"]:
        path, _ = resolve_project_relative_path(project_root, candidate["path"], field="candidate.path")
        if path in candidates:
            _fail("migration_candidate_duplicate", "Migration candidate paths must be unique.", path=path)
        normalized = {**candidate, "path": path}
        candidates[path] = normalized
        by_kind.setdefault(candidate["kind"], []).append(normalized)

    candidate_set_issues = []
    for kind in ("plan", "task_index", "execution_index"):
        if len(by_kind.get(kind, [])) != 1:
            candidate_set_issues.append(kind)
    task_ids = [item["task_id"] for item in by_kind.get("task_item", [])]
    if not task_ids or len(task_ids) != len(set(task_ids)):
        candidate_set_issues.append("task_item")
    relationship_results: list[dict[str, object]] = []
    if candidate_set_issues:
        relationship_results.append({
            "name": "candidate_set", "status": "failed", "code": "migration_candidate_set_incomplete",
            "message": "A complete candidate set requires one Plan, TASK index, execution index, and unique TASK items.",
        })
    else:
        relationship_results.append({"name": "candidate_set", "status": "passed"})

    rendered = {path: render_json_contract(item["content"]) for path, item in candidates.items()}
    validator_results: list[dict[str, object]] = []
    collection_validation = None
    execution_value = None
    if not candidate_set_issues:
        plan = by_kind["plan"][0]
        index = by_kind["task_index"][0]
        execution = by_kind["execution_index"][0]
        items = {item["task_id"]: rendered[item["path"]] for item in by_kind["task_item"]}
        check, _ = _check("plan", lambda: validate_plan_contract(
            rendered[plan["path"]], source=plan["path"], actual_plan_path=plan["path"],
            project_root=project_root, user_config_root=user_config_root,
            skill_roots=skill_roots, _allow_task_index=True,
        ))
        validator_results.append(check)
        check, collection_validation = _check("task_collection", lambda: validate_task_collection_contract(
            rendered[index["path"]], items, source=index["path"], actual_index_path=index["path"],
            project_root=project_root, user_config_root=user_config_root, validate_file_state=False,
            skill_roots=skill_roots, _source_plan_raw=rendered[plan["path"]],
        ))
        validator_results.append(check)
        check, execution_value = _check("execution_index", lambda: validate_execution_index(
            rendered[execution["path"]], source=execution["path"],
        ))
        validator_results.append(check)

        artifacts = plan["content"].get("artifacts", {})
        paths_match = (
            artifacts.get("plan") == plan["path"]
            and artifacts.get("task") == index["path"]
            and artifacts.get("execution", "") + "/index.json" == execution["path"]
        )
        relationship_results.append(
            {"name": "artifact_paths", "status": "passed"} if paths_match else
            {"name": "artifact_paths", "status": "failed", "code": "migration_artifact_path_mismatch",
             "message": "Candidate paths do not match the Plan artifact routing."}
        )
        if collection_validation is not None and execution_value is not None:
            execution_content = execution["content"]
            rows = {row.get("id"): row for row in execution_content.get("tasks", []) if isinstance(row, dict)}
            ids = collection_validation["task_ids"]
            bound = (
                execution_content.get("requirement_id") == collection_validation["requirement_id"]
                and execution_content.get("task_spec_id") == collection_validation["spec_id"]
                and execution_content.get("task_collection_sha256") == collection_validation["task_collection_sha256"]
                and execution_content.get("task_index_sha256") == collection_validation["task_index_sha256"]
                and set(rows) == set(ids)
                and all(rows[task_id].get("task_item_sha256") == collection_validation["task_item_sha256"][task_id] for task_id in ids)
            )
            relationship_results.append(
                {"name": "execution_binding", "status": "passed"} if bound else
                {"name": "execution_binding", "status": "failed", "code": "migration_execution_binding_mismatch",
                 "message": "The execution index does not bind to the candidate TASK collection."}
            )
        else:
            relationship_results.append({
                "name": "execution_binding", "status": "failed", "code": "migration_execution_binding_not_checked",
                "message": "Execution binding requires valid TASK and execution candidates.",
            })

    unresolved = sorted(item["id"] for item in request["semantic_decisions"] if item["resolution"] is None)
    all_paths = sorted(set(sources) | set(candidates))
    diffs = [_diff(path, sources.get(path), rendered.get(path)) for path in all_paths]
    evidence = {
        "request": request,
        "source_sha256": {path: raw_sha256(raw) for path, raw in sorted(sources.items())},
        "candidate_sha256": {path: raw_sha256(raw) for path, raw in sorted(rendered.items())},
        "validator_results": validator_results,
        "relationship_results": relationship_results,
        "unresolved_items": unresolved,
    }
    ready = not unresolved and all(
        item["status"] == "passed" for item in validator_results + relationship_results
    )
    return SpecificationMigrationPreviewContract.model_validate({
        "schema": "work-spec-migration-preview/v1", "status": "ready" if ready else "blocked",
        "documents": all_paths, "diffs": diffs, "validator_results": validator_results,
        "relationship_results": relationship_results, "unresolved_items": unresolved,
        "fingerprint": canonical_json_sha256(evidence), "writable_ready": ready,
    }).to_canonical_dict()


def _migration_context(request: dict[str, Any], project_root: Path) -> tuple[dict[str, bytes], dict[str, bytes], str]:
    sources: dict[str, bytes] = {}
    for evidence in request["sources"]:
        path, resolved = resolve_project_relative_path(project_root, evidence["path"], field="source.path")
        sources[path] = read_raw(resolved)
    candidates = {
        resolve_project_relative_path(project_root, row["path"], field="candidate.path")[0]: render_json_contract(row["content"])
        for row in request["candidates"]
    }
    plan = next(row["content"] for row in request["candidates"] if row["kind"] == "plan")
    execution_dir = plan["artifacts"]["execution"]
    return sources, candidates, execution_dir


def _transaction(
    request: dict[str, Any], preview: dict[str, Any], sources: dict[str, bytes],
    candidates: dict[str, bytes], execution_dir: str, project_root: Path,
) -> dict[str, Any]:
    files = []
    for path in sorted(set(sources) | set(candidates)):
        before, after = sources.get(path), candidates.get(path)
        row: dict[str, Any] = {"phase": 50 if after is None else 10, "path": path}
        if before is None:
            row.update(operation="add", after=encode_snapshot(after))
        elif after is None:
            row.update(operation="remove", before=encode_snapshot(before))
        else:
            row.update(operation="replace", before=encode_snapshot(before), after=encode_snapshot(after))
        files.append(row)
    files.sort(key=lambda row: (row["phase"], row["path"]))
    task_ids = sorted(row["task_id"] for row in request["candidates"] if row["kind"] == "task_item")
    metadata = {
        "request": {"migration": request, "preview_fingerprint": preview["fingerprint"]},
        "artifacts": next(row["content"]["artifacts"] for row in request["candidates"] if row["kind"] == "plan"),
        "affected_task_ids": task_ids,
        "history_sha256": execution_history_fingerprints(project_root, execution_dir),
        "source_sha256": {path: raw_sha256(raw) for path, raw in sorted(sources.items())},
        "candidate_sha256": {path: raw_sha256(raw) for path, raw in sorted(candidates.items())},
    }
    approval = transaction_approval_sha256(files, metadata)
    return {
        "schema": "work-spec-transaction/v1",
        "transaction_id": "SPEC-MIGRATION-" + preview["fingerprint"][:12].upper(),
        "approval_sha256": approval, "state": "prepared", "published_count": 0,
        "metadata": metadata, "files": files,
    }


def _post_validate(
    request: dict[str, Any], *, project_root: Path, user_config_root: str, skill_roots,
) -> dict[str, Any]:
    current = copy.deepcopy(request)
    current["sources"] = [
        {"path": row["path"], "raw_sha256": raw_sha256(render_json_contract(row["content"]))}
        for row in current["candidates"]
    ]
    return preview_specification_migration(
        render_json_contract(current), project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
    )


def publish_specification_migration(
    raw_request: bytes,
    *,
    project_root: Path,
    user_config_root: str,
    skill_roots=None,
    operation: str = "apply",
    approved_sha256: str,
) -> dict[str, object]:
    request = SpecificationMigrationPreviewRequestContract.parse_json_bytes(
        raw_request, source="specification migration publication",
    ).to_canonical_dict()
    plan = next((row["content"] for row in request["candidates"] if row["kind"] == "plan"), {})
    execution_dir = plan.get("artifacts", {}).get("execution")
    if not isinstance(execution_dir, str):
        _fail("migration_execution_directory", "A candidate execution directory is required.")
    journal_relative = execution_dir + "/.work-spec-migration-" + approved_sha256[:12].upper() + ".json"
    marker_relative = journal_relative + ".done"
    if operation == "apply":
        preview = preview_specification_migration(
            raw_request, project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        )
        if not preview["writable_ready"]:
            _fail("migration_not_writable", "The migration preview is not ready for publication.")
        if preview["fingerprint"] != approved_sha256:
            _fail("migration_approval_changed", "The approved migration fingerprint changed.")
        sources, candidates, execution_dir = _migration_context(request, project_root)
        journal = _transaction(request, preview, sources, candidates, execution_dir, project_root)
    elif operation == "recover":
        journal = validate_spec_transaction(read_raw(storage_path(project_root, journal_relative)), source=journal_relative)
        saved = journal["metadata"]["request"]
        if saved != {"migration": request, "preview_fingerprint": approved_sha256}:
            _fail("migration_recovery_request_changed", "Recovery requires the identical approved migration request.")
        preview = {"fingerprint": approved_sha256}
        candidates = {
            row["path"]: render_json_contract(row["content"])
            for row in request["candidates"]
        }
    else:
        _fail("migration_operation", "Use apply or recover for migration publication.")

    execution_path = storage_path(project_root, execution_dir)
    execution_path.mkdir(parents=True, exist_ok=True)
    require_no_spec_update(project_root, execution_dir, ignored_record=journal_relative)
    require_idle_writer(project_root, execution_dir)
    with state_writer(project_root, execution_dir):
        require_no_spec_update(project_root, execution_dir, ignored_record=journal_relative)
        try:
            if operation == "apply":
                spec_transactions.write_journal(storage_path(project_root, journal_relative), journal)
            published = spec_transactions.publish_journal(project_root, journal_relative, marker_relative)
        except (OSError, WorkError) as error:
            raise WorkError(
                ExitCode.IO_FAILURE, "migration_interrupted",
                "Preserve the migration transaction and obtain recovery authorization.",
                {"recovery_required": True, "record": journal_relative},
            ) from error

    for path in sorted(set(journal["metadata"]["source_sha256"]) | set(journal["metadata"]["candidate_sha256"])):
        target = storage_path(project_root, path)
        expected = journal["metadata"]["candidate_sha256"].get(path)
        if expected is None:
            if target.exists():
                _fail("migration_post_write", "A removed source artifact still exists.", path=path)
        elif raw_sha256(read_raw(target)) != expected:
            _fail("migration_post_write", "An installed migration artifact differs from approval.", path=path)
    validated = _post_validate(
        request, project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
    )
    if not validated["writable_ready"]:
        _fail("migration_post_validation", "Installed migration artifacts failed current validation.")
    return SpecificationMigrationPublicationContract.model_validate({
        "schema": "work-spec-migration-publication/v1",
        "status": "recovered" if operation == "recover" else "updated",
        "fingerprint": approved_sha256,
        "transaction_approval_sha256": journal["approval_sha256"],
        "journal": journal_relative, "completion_marker": marker_relative,
        "documents": validated["documents"], "publication_status": published["status"],
        "validator_results": validated["validator_results"],
        "relationship_results": validated["relationship_results"],
    }).to_canonical_dict()
