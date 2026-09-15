"""Verify the exact installed result of one completed specification update."""
from __future__ import annotations

import re
from pathlib import Path

from ..contracts.task_diagnostics import Diagnostics, diagnose_task_contract
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import validate_artifact_paths
from ..foundation.spec_update import completion_marker_matches, require_idle_writer, storage_path
from .migration_preflight import _git_visibility, _transactions
from .specification import _history, _json, _prepare


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _same(actual, expected, code: str, message: str, **details: object) -> bool:
    if actual != expected:
        _fail(code, message, **details)
    return True


def _record(raw: bytes, record_id: str, requirement_id: str, artifacts: dict[str, str]):
    value = parse_json_contract(raw, source="specification journal")
    request = value.get("request") if isinstance(value, dict) else None
    if isinstance(request, dict) and request.get("schema") == "work-spec-migration-request/v1":
        _fail("spec_verify_not_specification", "The selected journal is not an ordinary specification update.")
    record = strict_keys(
        value, location="specification journal",
        required={"schema", "record_id", "request", "before", "after", "history_sha256", "affected_task_ids"},
    )
    if record["schema"] != "work-spec-update-record/v1" or record["record_id"] != record_id:
        _fail("spec_verify_record_identity", "Journal schema or record ID differs from the selected update.")
    if _json(record) != raw:
        _fail("spec_verify_record_canonical", "The specification journal is not canonical JSON.")
    request = record["request"]
    if not isinstance(request, dict) or request.get("schema") != "work-spec-update-request/v1":
        _fail("spec_verify_not_specification", "The selected journal is not an ordinary specification update.")
    for key in ("plan", "task"):
        value = request.get(key)
        if not isinstance(value, dict) or value.get("requirement_id") != requirement_id or value.get("artifacts") != artifacts:
            _fail("spec_verify_artifact_identity", "The journal belongs to another requirement or artifact routing.")
    for side in ("before", "after"):
        values = strict_keys(record[side], location="record." + side, required={"plan", "task", "index"})
        if any(not isinstance(value, str) for value in values.values()):
            _fail("spec_verify_record_bytes", "The journal must preserve original and installed artifact bytes.")
    if not isinstance(record["history_sha256"], dict):
        _fail("spec_verify_record_history", "The journal must contain an execution history fingerprint map.")
    return record


def verify_specification(raw_request: bytes, *, project_root: Path,
                         user_config_root: str, skill_roots=None) -> dict[str, object]:
    request = strict_keys(
        parse_json_contract(raw_request, source="specification verification request"),
        location="spec_verify", required={"schema", "requirement_id", "artifacts", "record_id"},
    )
    if request["schema"] != "work-spec-verification-request/v1":
        _fail("spec_verify_schema", "Use work-spec-verification-request/v1.")
    requirement_id = nonempty_string(request["requirement_id"], location="requirement_id")
    record_id = nonempty_string(request["record_id"], location="record_id")
    if not re.fullmatch(r"SPEC-UPDATE-\d{3}", record_id):
        _fail("spec_verify_record_id", "Use an exact SPEC-UPDATE-nnn record ID.")
    declared = request["artifacts"]
    artifacts = validate_artifact_paths(
        project_root, requirement_id, declared,
        actual_plan_path=declared.get("plan") if isinstance(declared, dict) else "",
    )
    execution = artifacts["execution"]
    journal_path = execution + "/.work-spec-update-" + record_id + ".json"
    paths = {
        "plan": artifacts["plan"], "task": artifacts["task"], "index": execution + "/index.json",
        "journal": journal_path, "completion": journal_path + ".done",
    }
    report = Diagnostics()
    report.check("writer_idle", lambda: require_idle_writer(project_root, execution))
    snapshots = {
        key: report.check("read:" + key, lambda path=path: read_raw(storage_path(project_root, path)), location=path)
        for key, path in paths.items()
    }
    journal_raw = snapshots["journal"]
    record = report.check(
        "journal_contract", lambda: _record(journal_raw, record_id, requirement_id, artifacts),
    ) if journal_raw is not None else report.skip("journal_contract", "read:journal")
    if journal_raw is not None and snapshots["completion"] is not None:
        report.check("completion_marker", lambda: _same(
            completion_marker_matches(journal_raw, snapshots["completion"]), True,
            "spec_verify_completion_mismatch", "The completion marker does not match the journal SHA-256.",
        ))
    else:
        report.skip("completion_marker", "read:journal", "read:completion")
    diagnostics = diagnose_task_contract(
        snapshots["task"], source="specification verification TASK", actual_task_path=artifacts["task"],
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        plan_path=artifacts["plan"], execution_dir=execution,
        _source_plan_raw=snapshots["plan"], _index_raw=snapshots["index"],
    )
    transactions = _transactions(project_root, execution, report)
    history = report.check("history_snapshot", lambda: _history(project_root, execution))
    if record is not None:
        for key in ("plan", "task", "index"):
            if snapshots[key] is None:
                report.skip("installed:" + key, "read:" + key)
            else:
                report.check("installed:" + key, lambda key=key: _same(
                    snapshots[key], record["after"][key].encode("utf-8"),
                    "spec_verify_state_changed", "The current artifact differs from this update's installed result.",
                    artifact=key,
                ))
        if history is not None:
            report.check("history_preserved", lambda: _same(
                history, record["history_sha256"], "spec_verify_history_changed",
                "Execution history differs from the specification update snapshot.",
            ))
        else:
            report.skip("history_preserved", "history_snapshot")

        def reproduce():
            rebuilt, _ = _prepare(
                record["request"], project_root=project_root, user_config_root=user_config_root,
                skill_roots=skill_roots, before=record["before"], migration=False,
            )
            return _same(
                _json(rebuilt), journal_raw, "spec_verify_record_mismatch",
                "The journal differs from the revalidated specification transaction.",
            )
        report.check("specification_evidence", reproduce)
    else:
        for name in ("installed_artifacts", "history_preserved", "specification_evidence"):
            report.skip(name, "journal_contract")
    git = _git_visibility(project_root, paths)
    for key, raw in snapshots.items():
        if raw is not None:
            report.check("unchanged:" + key, lambda key=key, raw=raw: _same(
                read_raw(storage_path(project_root, paths[key])), raw,
                "spec_verify_concurrent_change", "A verification source changed during inspection.", artifact=key,
            ))
    if history is not None:
        report.check("history_unchanged", lambda: _same(
            _history(project_root, execution), history,
            "spec_verify_concurrent_history", "Execution history changed during verification.",
        ))
    report.check("writer_idle_after", lambda: require_idle_writer(project_root, execution))
    verified = (
        diagnostics["normal_use_allowed"] and diagnostics["repair_mode"] == "review_required"
        and all(check["status"] == "passed" for check in report.checks)
        and all(item["completion"] == "completed" for item in transactions)
    )
    return {
        "schema": "work-spec-verification/v1", "status": "verified" if verified else "blocked",
        "verified": verified, "record_id": record_id, "requirement_id": requirement_id,
        "artifacts": artifacts, "verification_scope": "exact_specification_result",
        "journal_sha256": raw_sha256(journal_raw) if journal_raw is not None else None,
        "completion_status": report.status("completion_marker"), "checks": report.checks,
        "issues": report.issues, "task_diagnostics": diagnostics, "history_sha256": history,
        "transactions": transactions, "git": git, "execution_authorized": False,
        "next_step": "normal_execute_preflight" if verified else "review_failed_and_not_checked_checks",
    }
