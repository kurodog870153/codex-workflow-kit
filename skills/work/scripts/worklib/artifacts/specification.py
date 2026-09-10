"""Review and publish one coordinated Plan/TASK/index specification revision."""

from __future__ import annotations

import copy
import hashlib
import json
import os
import re
from pathlib import Path
from typing import Any

from ..contracts.execution_index import (
    build_initial_execution_index, derive_overall_status,
    render_execution_index, validate_execution_index,
)
from ..contracts.plan import render_plan_contract, validate_plan_contract
from ..contracts.task import render_task_contract, validate_task_contract
from ..contracts.validation import nonempty_string, sha256, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import validate_artifact_paths
from ..foundation.spec_update import require_no_spec_update, state_writer, storage_path
from ..skills.catalog import SkillRoot
from ..foundation.runtime import installed_work_root
from ..instructions.selection import build_instruction_selection
from ..execution.instructions import BASE_EXECUTE_REFERENCES, RECOVERY_REFERENCE


def _error(code: str, message: str, **details: object) -> WorkError:
    return WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details or None)


def _json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def _hash(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def _decode(raw: bytes) -> dict[str, Any]:
    return parse_json_contract(raw, source="specification update")


def _history(root: Path, execution: str) -> dict[str, str]:
    """Fingerprint immutable execution artifacts without copying their contents."""
    result = {}
    directory = storage_path(root, execution)
    for task_dir in directory.glob("TASK-*"):
        safe = storage_path(root, task_dir.relative_to(root).as_posix())
        if not safe.is_dir():
            raise _error("spec_update_history_layout", "An execution TASK entry must be a directory.")
        for current, directories, files in os.walk(safe, followlinks=False):
            for name in directories + files:
                path = storage_path(root, (Path(current) / name).relative_to(root).as_posix())
                if path.is_file():
                    result[path.relative_to(root).as_posix()] = _hash(path.read_bytes())
    return result


def _changes(before: dict[str, Any], after: dict[str, Any]) -> list[dict[str, Any]]:
    """Use whole top-level values so review evidence can be verified exactly."""
    result = []
    for key in sorted((before.keys() | after.keys()) - {"spec_id", "readiness", "changes"}):
        pointer = "/" + key.replace("~", "~0").replace("/", "~1")
        if key not in before:
            result.append({"operation": "add", "path": pointer, "after": after[key]})
        elif key not in after:
            result.append({"operation": "remove", "path": pointer, "before": before[key]})
        elif before[key] != after[key]:
            result.append({"operation": "replace", "path": pointer, "before": before[key], "after": after[key]})
    return result


def _index(
    old_plan: dict[str, Any], plan: dict[str, Any],
    old_task: dict[str, Any], task: dict[str, Any],
    old_index: dict[str, Any], validation: dict[str, object],
    *, migration: bool = False,
) -> tuple[dict[str, Any], list[str]]:
    old_tasks = {item["id"]: item for item in old_task["tasks"]}
    new_tasks = {item["id"]: item for item in task["tasks"]}
    if not old_tasks.keys() <= new_tasks.keys():
        raise _error("spec_update_task_removal", "Preserve existing TASK IDs and their history.")
    additions = new_tasks.keys() - old_tasks.keys()
    if additions and min(additions) <= max(old_tasks):
        raise _error("spec_update_task_id_reuse", "New TASK IDs must follow all existing TASK IDs.")
    if any(row["status"] == "in_progress" for row in old_index["tasks"]):
        raise _error("spec_update_active_task", "Close the active Attempt before revising specifications.")
    affected = {key for key in new_tasks if new_tasks[key] != old_tasks.get(key)}
    if (
        migration or plan != old_plan
        or task.get("execution_defaults") != old_task.get("execution_defaults")
        or task.get("decisions") != old_task.get("decisions")
    ):
        affected = set(new_tasks)
    while True:
        downstream = {
            key for key, value in new_tasks.items()
            if set(value.get("dependencies", [])) & affected
        }
        if downstream <= affected:
            break
        affected |= downstream
    generated = build_initial_execution_index(task, validation)
    result = copy.deepcopy(old_index)
    for key, value in generated.items():
        if key not in {"title", "tasks", "overall_status"}:
            result[key] = value
    rows = {row["id"]: row for row in old_index["tasks"]}
    result["tasks"] = []
    for fresh in generated["tasks"]:
        row = copy.deepcopy(rows.get(fresh["id"], fresh))
        row["skill_id"] = fresh["skill_id"]
        row["instructions_sha256"] = fresh["instructions_sha256"]
        if row["id"] in affected and row["status"] != "pending":
            if row["status"] == "cancelled":
                raise _error("spec_update_cancelled_task", "A cancelled TASK requires an explicit lifecycle decision.")
            row["status"] = "pending_retry" if "latest_attempt" in row else "blocked"
            row["status_reason"] = {"kind": "task_change", "ref": task["changes"][0]["id"]}
        result["tasks"].append(row)
    result["overall_status"] = derive_overall_status([row["status"] for row in result["tasks"]])
    return result, sorted(affected)


def _prepare(
    request: dict[str, Any], *, project_root: Path, user_config_root: str,
    skill_roots: list[SkillRoot] | None, before: dict[str, str] | None = None,
    migration: bool = False,
) -> tuple[dict[str, Any], dict[str, Path]]:
    fields = {"schema", "reason", "expected", "plan", "task"}
    if migration:
        fields.add("instruction_review")
    strict_keys(request, location="spec_update", required=fields)
    schema = "work-spec-migration-request/v1" if migration else "work-spec-update-request/v1"
    if request["schema"] != schema:
        raise _error("spec_update_schema", "Invalid specification update request schema.")
    if migration:
        review = strict_keys(request["instruction_review"], location="instruction_review",
                             required={"plan", "task", "execute"})
        for mode, evidence in review.items():
            nonempty_string(evidence, location="instruction_review." + mode)
    nonempty_string(request["reason"], location="reason")
    expected = strict_keys(request["expected"], location="expected",
                           required={"plan_sha256", "task_sha256", "index_sha256"})
    for key, value in expected.items():
        sha256(value, location="expected." + key)
    if not isinstance(request["plan"], dict) or not isinstance(request["task"], dict):
        raise _error("spec_update_candidate", "Complete Plan and TASK objects are required.")
    plan, task = request["plan"], request["task"]
    artifacts = validate_artifact_paths(project_root, plan.get("requirement_id"), plan.get("artifacts"),
                                       actual_plan_path=plan.get("artifacts", {}).get("plan", ""))
    paths = {key: storage_path(project_root, value) for key, value in artifacts.items()}
    paths["index"] = storage_path(project_root, artifacts["execution"] + "/index.json")
    if not all(paths[key].is_file() for key in ("plan", "task", "index")):
        raise _error("spec_update_existing_required", "This update requires an existing formal Plan, TASK and index.")
    original = {
        key: before[key].encode("utf-8") if before is not None else paths[key].read_bytes()
        for key in ("plan", "task", "index")
    }
    if {key + "_sha256": _hash(raw) for key, raw in original.items()} != expected:
        raise _error("spec_update_source_changed", "The reviewed source fingerprints no longer match.")
    old_plan, old_task, old_index = (_decode(original[key]) for key in ("plan", "task", "index"))
    for value in (old_plan, old_task, task):
        if value.get("artifacts") != artifacts or value.get("requirement_id") != plan["requirement_id"]:
            raise _error("spec_update_identity", "A specification update cannot rename or reroute a requirement.")
    options = dict(project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    validate_plan_contract(original["plan"], source="original Plan",
                           actual_plan_path=artifacts["plan"], _historical_work_sources=migration, **options)
    old_validation = validate_task_contract(
        original["task"], source="original TASK", actual_task_path=artifacts["task"],
        validate_file_state=False, _source_plan_raw=original["plan"], **options,
        _historical_work_sources=migration,
    )
    validate_execution_index(original["index"], source="original index")
    if "lock" in old_index:
        raise _error("spec_update_lock_present", "The execution index already contains a lock.")
    old_identity = build_initial_execution_index(old_task, old_validation)
    identity_keys = set(old_identity) - {"title", "tasks", "overall_status"}
    if any(old_index[key] != old_identity[key] for key in identity_keys):
        raise _error("spec_update_index_identity", "The execution index does not match the original TASK.")
    if [
        {key: row[key] for key in ("id", "skill_id", "instructions_sha256")}
        for row in old_index["tasks"]
    ] != [
        {key: row[key] for key in ("id", "skill_id", "instructions_sha256")}
        for row in old_identity["tasks"]
    ]:
        raise _error("spec_update_index_tasks", "The index TASK identities differ from the formal TASK.")
    rendered_plan, rendered_task = render_plan_contract(plan), render_task_contract(task)
    validate_plan_contract(rendered_plan, source="candidate Plan", actual_plan_path=artifacts["plan"], **options)
    validation = validate_task_contract(
        rendered_task, source="candidate TASK", actual_task_path=artifacts["task"],
        validate_file_state=False, _source_plan_raw=rendered_plan, **options,
    )
    next_number = int(old_task["spec_id"].rsplit("-", 1)[1]) + 1
    if next_number > 999 or task["spec_id"] != f"TASK-SPEC-{next_number:03d}":
        raise _error("spec_update_version", "Increment TASK spec exactly once.")
    edits = _changes(old_task, task)
    if not edits:
        raise _error("spec_update_unchanged", "A specification update must contain a substantive change.")
    previous_change = max((int(item["id"].rsplit("-", 1)[1]) for item in old_task.get("changes", [])), default=0)
    change = task["changes"]
    if (
        len(change) != 1 or change[0]["id"] != f"TASK-CHANGE-{previous_change + 1:03d}"
        or change[0]["reason"] != request["reason"] or change[0]["edits"] != edits
    ):
        raise _error("spec_update_change_evidence",
                     "Supply one next TASK-CHANGE with the request reason and exact sorted top-level edits.")
    old_changes = old_plan.get("changes", [])
    if plan != old_plan and (
        plan.get("changes", [])[:len(old_changes)] != old_changes
        or len(plan.get("changes", [])) <= len(old_changes)
    ):
        raise _error("spec_update_plan_evidence", "Append Plan change evidence without rewriting existing changes.")
    plan_change_ids = {item["id"] for item in plan.get("changes", [])}
    if not set(change[0].get("plan_change_ids", [])) <= plan_change_ids:
        raise _error("spec_update_plan_change_reference", "TASK changes reference unknown Plan changes.")
    index, affected = _index(old_plan, plan, old_task, task, old_index, validation, migration=migration)
    if not set(affected) <= {item.split("/", 1)[0] for item in change[0]["affected_ids"]}:
        raise _error("spec_update_affected_evidence", "Change evidence must cover every affected TASK.")
    rendered_index = render_execution_index(index)
    validate_execution_index(rendered_index, source="candidate index")
    record = {
        "schema": "work-spec-update-record/v1", "record_id": f"SPEC-UPDATE-{next_number:03d}",
        "request": request,
        "before": {key: raw.decode("utf-8") for key, raw in original.items()},
        "after": {"plan": rendered_plan.decode("utf-8"), "task": rendered_task.decode("utf-8"),
                  "index": rendered_index.decode("utf-8")},
        "history_sha256": _history(project_root, artifacts["execution"]),
        "affected_task_ids": affected,
    }
    if migration:
        # Bind approval to both normal and retry Execute guidance. Attempts keep
        # their historical fingerprints; a future preflight selects current ones.
        record["migration"] = {
            "plan_edits": _changes(old_plan, plan),
            "execute_instruction_selections": {
                row["id"]: {
                    mode: build_instruction_selection(
                        skill_root=installed_work_root(), mode="execute",
                        selected_paths=row["instruction_selection"]["selected_paths"],
                        reference_names=BASE_EXECUTE_REFERENCES + ([RECOVERY_REFERENCE] if mode == "retry" else []),
                    ) for mode in ("normal", "retry")
                } for row in task["tasks"]
            },
        }
    return record, paths


def _write(path: Path, raw: bytes) -> None:
    with path.open("xb") as output:
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())


def _complete_write(path: Path, target: bytes) -> None:
    """Authorized recovery may append a missing suffix, never overwrite bytes."""
    if not path.exists():
        _write(path, target)
        return
    with path.open("r+b") as output:
        current = output.read()
        if not target.startswith(current):
            raise _error("spec_update_partial_conflict", "Partial transaction bytes conflict with approval.")
        if current != target:
            output.write(target[len(current):])
            output.flush()
            os.fsync(output.fileno())


def _replace(
    path: Path, expected: bytes, target: bytes, temporary: Path, *, recover: bool = False,
) -> None:
    if recover:
        _complete_write(temporary, target)
    elif temporary.exists():
        if temporary.read_bytes() != target:
            raise _error("spec_update_temporary_changed", "Prepared specification bytes changed.")
    else:
        _write(temporary, target)
    if path.read_bytes() != expected:
        raise _error("spec_update_concurrent_change", "An artifact changed before publication.")
    os.replace(temporary, path)
    if path.read_bytes() != target:
        raise _error("spec_update_write_mismatch", "Published specification bytes differ from the approved candidate.")


def update_specification(
    raw_request: bytes, *, project_root: Path, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
    operation: str = "validate", approved_sha256: str | None = None,
    migration: bool = False,
) -> dict[str, object]:
    if operation not in {"validate", "apply", "recover"}:
        raise _error("spec_update_operation", "Unknown specification update operation.")
    request = _decode(raw_request)
    # Resolve storage before accessing any journal or formal artifact.
    plan = request.get("plan")
    if not isinstance(plan, dict):
        raise _error("spec_update_candidate", "A complete Plan object is required.")
    artifacts = validate_artifact_paths(project_root, plan.get("requirement_id"), plan.get("artifacts"),
                                       actual_plan_path=plan.get("artifacts", {}).get("plan", ""))
    execution = artifacts["execution"]
    saved = None
    partial_journal = None
    before = None
    if operation == "recover":
        spec = request.get("task", {}).get("spec_id", "")
        if not isinstance(spec, str) or not re.fullmatch(r"TASK-SPEC-\d{3}", spec):
            raise _error("spec_update_version", "A valid target TASK spec is required.")
        journal = storage_path(project_root, execution + "/.work-spec-update-SPEC-UPDATE-" + spec[-3:] + ".json")
        journal_raw = journal.read_bytes()
        try:
            saved = _decode(journal_raw)
        except WorkError:
            partial_journal = journal_raw
        if saved is not None and _json(saved) != journal_raw:
            saved, partial_journal = None, journal_raw
        if saved is not None:
            if saved.get("request") != request:
                raise _error("spec_update_recovery_request", "Recovery requires the identical approved request.")
            before = strict_keys(saved.get("before"), location="record.before", required={"plan", "task", "index"})
            if any(not isinstance(raw, str) for raw in before.values()):
                raise _error("spec_update_recovery_record", "The complete original artifact bytes are required.")
    else:
        require_no_spec_update(project_root, execution)
        before = None
    record, paths = _prepare(request, project_root=project_root, user_config_root=user_config_root,
                             skill_roots=skill_roots, before=before, migration=migration)
    record_raw = _json(record)
    approval = _hash(record_raw)
    result = {
        "schema": "work-spec-update/v1", "status": "valid",
        "requirement_id": plan["requirement_id"], "record_id": record["record_id"],
        "approved_sha256": approval, "affected_task_ids": record["affected_task_ids"],
        "artifacts": artifacts, "candidate": {key: _decode(value.encode("utf-8")) for key, value in record["after"].items()},
        "file_readiness": "requires_execute_preflight",
    }
    if migration:
        result["migration"] = record["migration"]
        result["instruction_review"] = request["instruction_review"]
    if operation == "validate":
        return result
    sha256(approved_sha256, location="approved_sha256")
    if approved_sha256 != approval:
        raise _error("spec_update_approval_changed", "The candidate, sources or execution history changed after approval.")
    if partial_journal is not None and not record_raw.startswith(partial_journal):
        raise _error("spec_update_partial_conflict", "The incomplete record is not a prefix of the approved transaction.")
    if saved is not None and saved != record:
        raise _error("spec_update_recovery_changed", "The revalidated recovery differs from the preserved record.")
    journal = storage_path(project_root, execution + "/.work-spec-update-" + record["record_id"] + ".json")
    done = storage_path(project_root, journal.relative_to(project_root).as_posix() + ".done")
    before_raw = {key: value.encode("utf-8") for key, value in record["before"].items()}
    after_raw = {key: value.encode("utf-8") for key, value in record["after"].items()}
    locked = _decode(before_raw["index"])
    locked["lock"] = {"kind": "spec_update", "record": record["record_id"]}
    locked_raw = render_execution_index(locked)
    validate_execution_index(locked_raw, source="specification lock")
    current = {key: paths[key].read_bytes() for key in before_raw}
    allowed = {
        "plan": {before_raw["plan"], after_raw["plan"]},
        "task": {before_raw["task"], after_raw["task"]},
        "index": {before_raw["index"], locked_raw, after_raw["index"]},
    }
    if any(current[key] not in values for key, values in allowed.items()):
        raise _error("spec_update_recovery_conflict", "A current artifact is outside the approved transaction.")
    if current["index"] == before_raw["index"] and any(current[key] != before_raw[key] for key in ("plan", "task")):
        raise _error("spec_update_recovery_order", "Specification bytes changed without the matching lock.")
    if current["index"] == after_raw["index"] and current != after_raw:
        raise _error("spec_update_recovery_order", "An unlocked index cannot precede the complete specification.")
    if operation == "apply" and current != before_raw:
        raise _error("spec_update_source_changed", "Formal artifacts changed before update.")
    for entry in paths["execution"].glob(".work-*.tmp"):
        raise _error("spec_update_other_transaction", "Another execution transaction requires recovery.", path=str(entry))
    if operation == "recover":
        for entry in paths["execution"].glob(".work-spec-update-*.json"):
            if entry != journal:
                marker = storage_path(project_root, entry.relative_to(project_root).as_posix() + ".done")
                if not marker.is_file() or marker.read_bytes() != _hash(entry.read_bytes()).encode("ascii") + b"\n":
                    raise _error("spec_update_other_transaction", "Another specification update requires recovery.")
    with state_writer(project_root, execution):
        # Recheck after acquiring the same OS mutex used by Execute mutations.
        if operation == "apply":
            require_no_spec_update(project_root, execution)
        if any(paths[key].read_bytes() != current[key] for key in current):
            raise _error("spec_update_source_changed", "Artifacts changed before exclusive publication.")
        if _history(project_root, execution) != record["history_sha256"]:
            raise _error("spec_update_history_changed", "History changed before exclusive publication.")
        try:
            if operation == "apply":
                _write(journal, record_raw)
            elif partial_journal is not None:
                _complete_write(journal, record_raw)
            elif journal.read_bytes() != record_raw:
                raise _error("spec_update_recovery_changed", "The preserved transaction changed.")
            marker = approval.encode("ascii") + b"\n"
            if done.exists():
                observed = done.read_bytes()
                if current != after_raw or not marker.startswith(observed):
                    raise _error("spec_update_completion_conflict", "The completion marker conflicts with the transaction.")
                _complete_write(done, marker)
                result["status"] = "already_completed" if observed == marker else "recovered"
                return result
            recovering = operation == "recover"
            if current["index"] == before_raw["index"]:
                temporary = storage_path(project_root, journal.relative_to(project_root).as_posix() + ".lock")
                _replace(paths["index"], before_raw["index"], locked_raw, temporary, recover=recovering)
                current["index"] = locked_raw
            for key in ("plan", "task"):
                if current[key] != after_raw[key]:
                    relative = paths[key].relative_to(project_root).as_posix() + "." + record["record_id"] + ".tmp"
                    _replace(paths[key], before_raw[key], after_raw[key], storage_path(project_root, relative), recover=recovering)
            if _history(project_root, execution) != record["history_sha256"]:
                raise _error("spec_update_history_changed", "Execution history changed during specification publication.")
            if any(paths[key].read_bytes() != after_raw[key] for key in ("plan", "task")):
                raise _error("spec_update_post_write", "The installed specification differs from approval.")
            if current["index"] != after_raw["index"]:
                temporary = storage_path(project_root, journal.relative_to(project_root).as_posix() + ".index")
                _replace(paths["index"], locked_raw, after_raw["index"], temporary, recover=recovering)
            _write(done, marker)
        except (OSError, WorkError) as error:
            raise WorkError(
                ExitCode.IO_FAILURE, "spec_update_interrupted",
                "Preserve the specification transaction and lock; obtain separate recovery authorization.",
                {"recovery_required": True, "record": journal.relative_to(project_root).as_posix()},
            ) from error
    result["status"] = "recovered" if operation == "recover" else "updated"
    return result
