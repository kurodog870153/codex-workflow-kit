"""Explicitly reviewed TASK repair, including incomplete-but-readable documents."""

from __future__ import annotations

import base64
import copy
import difflib
import hashlib
import json
from pathlib import Path

from ..contracts.execution_index import build_initial_execution_index, derive_overall_status, render_execution_index
from ..contracts.task import validate_task_contract
from ..contracts.task_diagnostics import _json_document, diagnose_task_contract
from ..contracts.task_ordering import order_task_contract
from ..contracts.validation import nonempty_string, sha256, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import decode_utf8, read_raw
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import validate_artifact_paths
from ..foundation.spec_update import require_idle_writer, require_no_spec_update, state_writer, storage_path
from .specification import _complete_write, _history, _replace, _write


def _fail(code, message, **details):
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _hash(raw):
    return hashlib.sha256(raw).hexdigest()


def _json(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")


def _encode(raw):
    return base64.b64encode(raw).decode("ascii")


def _decode(value):
    try:
        return base64.b64decode(value, validate=True)
    except (ValueError, TypeError):
        _fail("task_repair_record_bytes", "The preserved original bytes are invalid.")


def _preserve_shape(original, ordered):
    # Normal TASK ordering supplies nulls for some missing fields. A format
    # repair must never introduce those placeholders into a partial document.
    if isinstance(original, dict):
        return {key: _preserve_shape(original[key], value) for key, value in ordered.items() if key in original}
    if isinstance(original, list):
        return [_preserve_shape(before, after) for before, after in zip(original, ordered)]
    return ordered


def _render(task):
    rendered = render_json_contract(_preserve_shape(task, order_task_contract(task)))
    if _json_document(decode_utf8(rendered, source="candidate TASK"), rendered) != task:
        _fail("task_repair_candidate_changed", "Rendering changed candidate values.")
    return rendered


def _request(raw, root):
    value = strict_keys(parse_json_contract(raw, source="TASK repair request"),
                        location="task_repair",
                        required={"schema", "stage", "requirement_id", "artifacts", "expected", "decisions", "task"})
    if value["schema"] != "work-task-repair-request/v1" or value["stage"] not in ("format", "complete"):
        _fail("task_repair_schema", "Use work-task-repair-request/v1 and stage format or complete.")
    requirement = nonempty_string(value["requirement_id"], location="requirement_id")
    artifacts = value["artifacts"]
    actual = artifacts.get("plan") if isinstance(artifacts, dict) else ""
    paths = validate_artifact_paths(root, requirement, artifacts, actual_plan_path=actual)
    if paths != artifacts:
        _fail("task_repair_paths", "Repair requests must use normalized project-relative paths.")
    expected = strict_keys(value["expected"], location="expected", required={"plan_sha256", "task_sha256", "index_sha256"})
    for key, fingerprint in expected.items():
        sha256(fingerprint, location="expected." + key)
    decisions = value["decisions"]
    if not isinstance(decisions, list) or not decisions:
        _fail("task_repair_decisions", "Retain the user's repair-direction decisions before preview.")
    for item in decisions:
        item = strict_keys(item, location="decisions[]", required={"location", "decision"})
        location = nonempty_string(item["location"], location="decisions[].location")
        if not location.startswith("/"):
            _fail("task_repair_decision_location", "Use a JSON Pointer or / for whole-document decisions.")
        nonempty_string(item["decision"], location="decisions[].decision")
    task = value["task"]
    if not isinstance(task, dict):
        _fail("task_repair_candidate", "An explicit candidate TASK object is required.")
    if task.get("schema") != "work-task/v1" or task.get("requirement_id") != requirement or task.get("artifacts") != paths:
        _fail("task_repair_identity", "Repair must preserve the confirmed requirement and artifact routing.")
    return value


def _diff(before, after):
    try:
        original = before.decode("utf-8", errors="strict")
        representation = "utf-8"
    except UnicodeDecodeError:
        # Byte escapes preserve undecodable evidence without guessing an encoding.
        original = repr(before) + "\n"
        representation = "python_bytes_repr"
    return {
        "before_representation": representation,
        "before": original,
        "after": after.decode("utf-8"),
        "unified": "".join(difflib.unified_diff(
            original.splitlines(keepends=True), after.decode("utf-8").splitlines(keepends=True),
            fromfile="original TASK", tofile="candidate TASK",
        )),
    }


def _prepare(request, *, project_root, user_config_root, skill_roots,
             before=None, ignored_record=None, writer_owned=False):
    artifacts = request["artifacts"]
    if not writer_owned:
        require_idle_writer(project_root, artifacts["execution"])
    paths = {key: storage_path(project_root, value) for key, value in artifacts.items()}
    paths["index"] = storage_path(project_root, artifacts["execution"] + "/index.json")
    if before is None:
        before = {key: read_raw(paths[key]) for key in ("plan", "task", "index")}
    if {key + "_sha256": _hash(raw) for key, raw in before.items()} != request["expected"]:
        _fail("task_repair_source_changed", "The original artifact fingerprints changed.")
    require_no_spec_update(project_root, artifacts["execution"], ignored_record=ignored_record)
    options = dict(
        source="TASK repair", actual_task_path=artifacts["task"],
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        plan_path=artifacts["plan"], execution_dir=artifacts["execution"],
        _source_plan_raw=before["plan"], _index_raw=before["index"],
        _ignored_repair_record=ignored_record,
    )
    original_report = diagnose_task_contract(before["task"], **options)
    if original_report["repair_mode"] != "review_required":
        _fail("task_repair_diagnose_only", "Execution state permits diagnosis only; no repair preview is available.",
              task_diagnostics=original_report)
    task = request["task"]
    candidate = _render(task)
    candidate_report = diagnose_task_contract(candidate, **options)
    if candidate_report["format_status"] != "passed":
        _fail("task_repair_format", "The candidate must pass encoding, JSON and canonical text checks.",
              task_diagnostics=candidate_report)
    after = {**before, "task": candidate}
    affected = []
    if request["stage"] == "complete":
        validation = validate_task_contract(
            candidate, source="TASK repair", actual_task_path=artifacts["task"],
            project_root=project_root, user_config_root=user_config_root,
            skill_roots=skill_roots, validate_file_state=False, _source_plan_raw=before["plan"],
        )
        old_index = parse_json_contract(before["index"], source="original index")
        fresh = build_initial_execution_index(task, validation)
        old_rows = {row["id"]: row for row in old_index["tasks"]}
        if list(old_rows) != [row["id"] for row in fresh["tasks"]]:
            _fail("task_repair_task_identity", "Repair must preserve existing TASK IDs and their history.")
        try:
            original_task = _json_document(decode_utf8(before["task"], source="original TASK"), before["task"])
        except WorkError:
            original_task = None
        # Unknown or changed meaning cannot retain historical completion claims.
        semantic_change = original_task != task
        result = copy.deepcopy(old_index)
        for key, value in fresh.items():
            if key not in {"title", "tasks", "overall_status"}:
                result[key] = value
        rows = []
        for identity in fresh["tasks"]:
            row = copy.deepcopy(old_rows[identity["id"]])
            changed = semantic_change or any(row[key] != identity[key] for key in ("skill_id", "instructions_sha256"))
            row.update({key: identity[key] for key in ("skill_id", "instructions_sha256")})
            if changed:
                affected.append(row["id"])
                if row["status"] == "cancelled":
                    _fail("task_repair_cancelled", "A changed cancelled TASK needs a separate lifecycle decision.")
                if row["status"] != "pending":
                    row["status"] = "pending_retry" if "latest_attempt" in row else "blocked"
                    row["status_reason"] = {"kind": "task_change", "ref": "TASK-REPAIR-" + _hash(_json(request))}
            rows.append(row)
        result["tasks"] = rows
        result["overall_status"] = derive_overall_status([row["status"] for row in rows])
        after["index"] = render_execution_index(result)
        candidate_report = diagnose_task_contract(candidate, **{**options, "_index_raw": after["index"]})
        if not candidate_report["normal_use_allowed"]:
            _fail("task_repair_incomplete", "Complete repair must pass the contract and all source bindings.",
                  task_diagnostics=candidate_report)
    if after == before:
        _fail("task_repair_no_change", "The request makes no repair change.")
    record = {
        "schema": "work-task-repair-record/v1", "request": request,
        "before": {key: _encode(raw) for key, raw in before.items()},
        "after": {key: _encode(raw) for key, raw in after.items()},
        "history_sha256": _history(project_root, artifacts["execution"]),
        "affected_task_ids": affected,
        "original_diagnostics": original_report, "candidate_diagnostics": candidate_report,
    }
    return record, paths, before, after


def _preview(record):
    before = {key: _decode(value) for key, value in record["before"].items()}
    after = {key: _decode(value) for key, value in record["after"].items()}
    return {
        "schema": "work-task-repair/v1", "status": "preview",
        "stage": record["request"]["stage"], "approved_sha256": _hash(_json(record)),
        "artifacts": record["request"]["artifacts"], "decisions": record["request"]["decisions"],
        "affected_task_ids": record["affected_task_ids"],
        "original_bytes_base64": record["before"],
        "diffs": {key: _diff(before[key], after[key]) for key in ("task", "index") if before[key] != after[key]},
        "task_diagnostics": record["candidate_diagnostics"],
        "file_readiness": "requires_execute_preflight",
    }


def repair_task(raw_request, *, project_root: Path, user_config_root: str,
                skill_roots=None, operation="validate", approved_sha256=None):
    if operation not in {"validate", "apply", "recover"}:
        _fail("task_repair_operation", "Unknown repair operation.")
    request = _request(raw_request, project_root)
    artifacts = request["artifacts"]
    execution = artifacts["execution"]
    saved = partial = None
    before = None
    ignored = None
    if operation != "validate":
        sha256(approved_sha256, location="approved_sha256")
    if operation == "recover":
        ignored = execution + "/.work-task-repair-" + approved_sha256 + ".json"
        journal_raw = read_raw(storage_path(project_root, ignored))
        try:
            saved = json.loads(journal_raw)
        except (ValueError, UnicodeError):
            partial = journal_raw
        if saved is not None:
            if _json(saved) != journal_raw or _hash(journal_raw) != approved_sha256 or saved.get("request") != request:
                _fail("task_repair_recovery_changed", "Recovery requires the exact approved request and record.")
            before = {key: _decode(value) for key, value in strict_keys(
                saved.get("before"), location="record.before", required={"plan", "task", "index"},
            ).items()}
    record, paths, before, after = _prepare(
        request, project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots, before=before, ignored_record=ignored,
    )
    result = _preview(record)
    approval = result["approved_sha256"]
    if operation == "validate":
        return result
    if approval != approved_sha256 or (saved is not None and saved != record):
        _fail("task_repair_approval_changed", "Sources, decisions, candidates or diagnostic evidence changed after review.")
    raw_record = _json(record)
    if partial is not None and not raw_record.startswith(partial):
        _fail("task_repair_partial_conflict", "The partial journal is outside the approved transaction.")
    relative = execution + "/.work-task-repair-" + approval + ".json"
    journal = storage_path(project_root, relative)
    done = storage_path(project_root, relative + ".done")
    marker = approval.encode("ascii") + b"\n"
    with state_writer(project_root, execution):
        # Both source snapshots and all live blockers are rechecked under the
        # same OS mutex used by Execute and specification transactions.
        rechecked, _, _, _ = _prepare(
            request, project_root=project_root, user_config_root=user_config_root,
            skill_roots=skill_roots, before=before if operation == "recover" else None,
            ignored_record=ignored, writer_owned=True,
        )
        if rechecked != record:
            _fail("task_repair_source_changed", "Repair evidence changed before publication.")
        current = {key: read_raw(paths[key]) for key in before}
        if operation == "apply" and current != before:
            _fail("task_repair_source_changed", "The original artifacts changed before publication.")
        if any(current[key] not in (before[key], after[key]) for key in before):
            _fail("task_repair_recovery_conflict", "Current bytes are outside the approved transaction.")
        if current["index"] != before["index"] and current["task"] != after["task"]:
            _fail("task_repair_recovery_order", "The index cannot precede the repaired TASK.")
        if done.exists():
            observed = read_raw(done)
            if current != after or not marker.startswith(observed):
                _fail("task_repair_completion_conflict", "Completion evidence conflicts with the transaction.")
            _complete_write(done, marker)
            result["status"] = "already_completed" if observed == marker else "recovered"
            return result
        try:
            if operation == "apply":
                _write(journal, raw_record)
            elif partial is not None:
                _complete_write(journal, raw_record)
            elif read_raw(journal) != raw_record:
                _fail("task_repair_recovery_changed", "The journal changed before recovery.")
            for key in ("task", "index"):
                if current[key] == after[key]:
                    continue
                # Keep temporaries beside their target for same-volume replace.
                temporary = storage_path(project_root, paths[key].relative_to(project_root).as_posix() + ".repair-" + approval + ".tmp")
                _replace(paths[key], before[key], after[key], temporary, recover=operation == "recover")
            if any(read_raw(paths[key]) != after[key] for key in after) or _history(project_root, execution) != record["history_sha256"]:
                _fail("task_repair_post_write", "Installed artifacts or execution history changed.")
            final = diagnose_task_contract(
                after["task"], source="TASK repair", actual_task_path=artifacts["task"],
                project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
                plan_path=artifacts["plan"], execution_dir=execution,
                _source_plan_raw=after["plan"], _index_raw=after["index"], _ignored_repair_record=relative,
            )
            if final != record["candidate_diagnostics"]:
                _fail("task_repair_post_validation", "Post-write validation differs from the reviewed result.")
            _write(done, marker)
        except (OSError, WorkError) as error:
            raise WorkError(
                ExitCode.IO_FAILURE, "task_repair_interrupted",
                "Preserve all transaction evidence and obtain separate recovery authorization.",
                {"record": relative, "recovery_required": True},
            ) from error
    result["status"] = "recovered" if operation == "recover" else "repaired"
    return result
