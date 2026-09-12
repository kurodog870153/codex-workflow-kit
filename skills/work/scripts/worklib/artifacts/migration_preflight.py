"""Read-only migration prerequisites; never creates candidates or approvals."""
from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path

from ..contracts.task_diagnostics import Diagnostics, diagnose_task_contract, _json_document
from ..contracts.validation import nonempty_string, strict_keys
from ..execution.instructions import BASE_EXECUTE_REFERENCES, RECOVERY_REFERENCE
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, decode_utf8, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import validate_artifact_paths
from ..foundation.runtime import installed_work_root
from ..foundation.spec_update import require_idle_writer, storage_path
from ..instructions.historical import stored_selection
from ..instructions.selection import build_instruction_selection
from .migration_transactions import verification_selection


def _hash(raw):
    return hashlib.sha256(raw).hexdigest()


def _fail(code, message, **details):
    raise WorkError(ExitCode.CONTRACT, code, message, details)


def _git_visibility(root, paths):
    def run(args):
        return subprocess.run(
            ["git", "--no-optional-locks", "-C", str(root), *args],
            stdin=subprocess.DEVNULL, capture_output=True, shell=False, timeout=10,
        )
    try:
        if run(["rev-parse", "--show-toplevel"]).returncode:
            return {"status": "not_checked", "reason": "not_a_git_worktree", "artifacts": {}}
        result = {}
        for key, path in paths.items():
            tracked = run(["ls-files", "--error-unmatch", "--", path])
            ignored = run(["check-ignore", "--quiet", "--", path])
            status = run(["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", path])
            if tracked.returncode not in (0, 1) or ignored.returncode not in (0, 1) or status.returncode:
                raise OSError("Git query failed")
            result[key] = {
                "path": path, "visibility": "tracked" if tracked.returncode == 0 else (
                    "ignored" if ignored.returncode == 0 else "untracked"),
                "has_changes": bool(status.stdout),
            }
        return {"status": "checked", "artifacts": result}
    except (OSError, subprocess.TimeoutExpired):
        return {"status": "not_checked", "reason": "git_unavailable_or_failed", "artifacts": {}}


def _transactions(root, execution, report):
    directory = report.check("transaction_directory", lambda: storage_path(root, execution))
    if directory is None:
        return []
    records = []
    for path in sorted([*directory.glob(".work-spec-update-*.json"), *directory.glob(".work-task-repair-*.json")]):
        relative = path.relative_to(root).as_posix()
        def inspect():
            raw = read_raw(storage_path(root, relative))
            marker = storage_path(root, relative + ".done")
            completed = marker.is_file() and read_raw(marker) == _hash(raw).encode("ascii") + b"\n"
            value = parse_json_contract(raw, source=relative)
            repair = path.name.startswith(".work-task-repair-")
            expected = "work-task-repair-record/v1" if repair else "work-spec-update-record/v1"
            if value.get("schema") != expected or not isinstance(value.get("request"), dict):
                _fail("migration_transaction_invalid", "The transaction schema or request is invalid.", path=relative)
            return {
                "path": relative, "raw_sha256": _hash(raw), "record_id": value.get("record_id"),
                "kind": "migration" if value["request"].get("schema") == "work-spec-migration-request/v1" else (
                    "repair" if repair else "specification"),
                "completion": "completed" if completed else "incomplete",
                "completion_evidence": "matching_sha256_marker" if completed else "missing_or_mismatched_marker",
            }
        value = report.check("transaction:" + relative, inspect, location=relative)
        records.append(value or {"path": relative, "completion": "unreadable", "kind": "unknown"})
    return records


def _source_comparison(selection, mode):
    stored_selection(selection)
    current = build_instruction_selection(
        skill_root=installed_work_root(), mode=mode, selected_paths=selection["selected_paths"],
        reference_names=selection["references"],
    )
    old = {(s["kind"], s["logical_name"]): s["canonical_sha256"] for s in selection["sources"]}
    new = {(s["kind"], s["logical_name"]): s["canonical_sha256"] for s in current["sources"]}
    return {
        "drift": current != selection,
        "changed_sources": [
            {"kind": key[0], "logical_name": key[1], "stored_sha256": old.get(key), "current_sha256": new.get(key)}
            for key in sorted(old.keys() | new.keys()) if old.get(key) != new.get(key)
        ],
        "stored_instructions_sha256": selection["instructions_sha256"],
        "current_selection": current, "source_order_changed": list(old) != list(new),
        "historical_content_verified": False,
    }


def migration_preflight(raw_request, *, project_root: Path, user_config_root: str, skill_roots=None):
    request = strict_keys(parse_json_contract(raw_request, source="migration preflight request"),
                          location="migration_preflight", required={"schema", "requirement_id", "artifacts"})
    if request["schema"] != "work-migration-preflight-request/v1":
        _fail("migration_preflight_schema", "Use work-migration-preflight-request/v1.")
    requirement = nonempty_string(request["requirement_id"], location="requirement_id")
    declared = request["artifacts"]
    artifacts = validate_artifact_paths(
        project_root, requirement, declared, actual_plan_path=declared.get("plan") if isinstance(declared, dict) else "",
    )
    report = Diagnostics()
    paths = {key: artifacts[key] for key in ("plan", "task")}
    paths["index"] = artifacts["execution"] + "/index.json"
    raw, fingerprints, documents = {}, {}, {}
    for key, relative in paths.items():
        value = report.check("read:" + key, lambda relative=relative: read_raw(storage_path(project_root, relative)), location=relative)
        raw[key] = value
        fingerprints[key] = {"raw_sha256": _hash(value) if value is not None else None}
        if value is None:
            report.skip("fingerprint:" + key, "read:" + key)
            documents[key] = report.skip("json:" + key, "read:" + key)
            continue
        fingerprints[key]["canonical_sha256"] = report.check(
            "fingerprint:" + key, lambda value=value, relative=relative: canonical_sha256(value, source=relative),
        )
        documents[key] = report.check("json:" + key, lambda value=value, key=key: _json_document(
            decode_utf8(value, source=key), value,
        ))
    for key, document in documents.items():
        if document is None:
            report.skip("identity:" + key, "json:" + key)
        else:
            def identity(document=document):
                if document.get("requirement_id") != requirement:
                    _fail("migration_requirement_mismatch", "The document belongs to another requirement.")
                return True
            report.check("identity:" + key, identity)
    options = dict(
        source="migration preflight TASK", actual_task_path=artifacts["task"],
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        plan_path=artifacts["plan"], execution_dir=artifacts["execution"],
        _source_plan_raw=raw["plan"], _index_raw=raw["index"],
    )
    current = diagnose_task_contract(raw["task"], **options)
    historical = diagnose_task_contract(raw["task"], _historical_work_sources=True, **options)
    report.check("writer_idle", lambda: require_idle_writer(project_root, artifacts["execution"]))
    transactions = _transactions(project_root, artifacts["execution"], report)
    selection = report.check("verification_selection", lambda: verification_selection(
        project_root, artifacts["execution"], raw,
    ))
    sources, execute = {}, {}
    plan, task, index = (documents[key] for key in ("plan", "task", "index"))
    if plan is not None:
        sources["plan"] = report.check("sources:plan", lambda: _source_comparison(plan.get("work_instruction_selection"), "plan"))
    else:
        report.skip("sources:plan", "json:plan")
    if task is not None and isinstance(task.get("tasks"), list):
        for number, row in enumerate(task["tasks"]):
            label = str(row.get("id", number)) if isinstance(row, dict) else str(number)
            selection = row.get("instruction_selection") if isinstance(row, dict) else None
            sources["task:" + str(number)] = report.check("sources:task:" + str(number), lambda selection=selection: _source_comparison(selection, "task"))
            def execute_sources(selection=selection):
                stored_selection(selection)
                return {mode: build_instruction_selection(
                    skill_root=installed_work_root(), mode="execute", selected_paths=selection["selected_paths"],
                    reference_names=BASE_EXECUTE_REFERENCES + ([RECOVERY_REFERENCE] if mode == "retry" else []),
                ) for mode in ("normal", "retry")}
            execute[str(number)] = {"task_id": label, "selections": report.check("sources:execute:" + str(number), execute_sources)}
    else:
        report.skip("sources:task", "json:task")
        report.skip("sources:execute", "json:task")
    states = index.get("tasks") if index is not None else None
    if isinstance(states, list):
        def lifecycle():
            if any(isinstance(row, dict) and row.get("status") == "cancelled" for row in states):
                _fail("migration_cancelled_task", "Migration affects every TASK; cancelled TASKs need a lifecycle decision.")
            return True
        report.check("migration_lifecycle", lifecycle)
    else:
        report.skip("migration_lifecycle", "json:index")
    for key, relative in paths.items():
        if raw[key] is not None:
            def unchanged(key=key, relative=relative):
                if read_raw(storage_path(project_root, relative)) != raw[key]:
                    _fail("migration_preflight_source_changed", "An artifact changed during preflight.", path=relative)
                return True
            report.check("unchanged:" + key, unchanged)
    ready = (historical["normal_use_allowed"] and historical["repair_mode"] == "review_required"
             and all(check["status"] == "passed" for check in report.checks)
             and all(record["completion"] == "completed" for record in transactions))
    drift = any(value and value["drift"] for value in sources.values())
    decisions = [{"kind": "instruction_review", "modes": ["plan", "task", "execute"],
                  "message": "Review specification meaning against current guidance before preparing candidates."}]
    binding = [issue for issue in historical["issues"] if issue["code"] == "source_plan_fingerprint_mismatch"]
    if binding:
        decisions.append({"kind": "source_plan_baseline", "evidence": binding,
                          "message": "Explicit baseline acceptance is required; preflight grants no binding exception."})
    return {
        "schema": "work-migration-preflight/v1",
        "status": "blocked" if not ready else ("review_required" if drift else "current"),
        "can_prepare_candidate": ready, "requirement_id": requirement, "artifacts": artifacts,
        "fingerprints": fingerprints, "task_spec_id": task.get("spec_id") if task else None,
        "current_diagnostics": current, "historical_diagnostics": historical,
        "checks": report.checks, "issues": report.issues, "instruction_sources": sources,
        "execute_instruction_selections": execute, "transactions": transactions,
        "verification_selection": selection,
        "execution_tasks": states, "required_decisions": decisions, "git": _git_visibility(project_root, paths),
        "historical_content_verified": False,
        "next_step": "resolve_reported_blockers" if not ready else "review_content_then_migrate_validate",
    }
