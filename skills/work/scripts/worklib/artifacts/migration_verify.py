"""Verify the exact installed result of one completed migration, read-only."""

from __future__ import annotations

import json
import copy
import re
from pathlib import Path

from ..contracts.attempt import validate_attempt_file
from ..contracts.correction import validate_correction_file
from ..contracts.task_diagnostics import Diagnostics, diagnose_task_contract
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import validate_artifact_paths, validate_execution_task_layout
from ..foundation.spec_update import require_idle_writer, storage_path
from .migration_preflight import _git_visibility, _transactions
from .specification import _hash, _history, _json, _prepare


def _fail(code, message, **details):
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def _same(actual, expected, code, message, **details):
    if actual != expected:
        _fail(code, message, **details)
    return True


def _history_snapshot(root, execution):
    try:
        return _history(root, execution)
    except OSError as error:
        raise WorkError(ExitCode.IO_FAILURE, "migration_verify_history_read",
                        "Execution history could not be inspected.") from error


def _history_without_transaction(root, execution, record_id):
    history = _history_snapshot(root, execution)
    prefix = execution + "/.work-spec-update-" + record_id
    return {
        path: value for path, value in history.items()
        if path != prefix + ".json" and path != prefix + ".json.done"
    }


def _record(raw, record_id, requirement, artifacts):
    record = strict_keys(
        parse_json_contract(raw, source="migration journal"), location="migration journal",
        required={"schema", "record_id", "request", "before", "after", "history_sha256", "affected_task_ids", "migration"},
    )
    if record["schema"] != "work-spec-update-record/v1" or record["record_id"] != record_id:
        _fail("migration_verify_record_identity", "Journal schema or record ID differs from the selected migration.")
    if _json(record) != raw:
        _fail("migration_verify_record_canonical", "The migration journal is not in its canonical representation.")
    request = record["request"]
    if not isinstance(request, dict) or request.get("schema") != "work-spec-migration-request/v1":
        _fail("migration_verify_not_migration", "The selected journal is not a migration.")
    for key in ("plan", "task"):
        value = request.get(key)
        if not isinstance(value, dict) or value.get("requirement_id") != requirement or value.get("artifacts") != artifacts:
            _fail("migration_verify_artifact_identity", "The journal belongs to another requirement or artifact routing.")
    for side in ("before", "after"):
        values = strict_keys(record[side], location="record." + side, required={"plan", "task", "index"})
        if any(not isinstance(value, str) for value in values.values()):
            _fail("migration_verify_record_bytes", "The journal must preserve all original and installed artifact bytes.")
    if not isinstance(record["history_sha256"], dict):
        _fail("migration_verify_record_history", "The journal must contain its execution history fingerprint map.")
    return record


def verify_migration(raw_request, *, project_root: Path, user_config_root: str, skill_roots=None):
    request = strict_keys(
        parse_json_contract(raw_request, source="migration verification request"),
        location="migration_verify", required={"schema", "requirement_id", "artifacts", "record_id"},
    )
    if request["schema"] != "work-migration-verify-request/v1":
        _fail("migration_verify_schema", "Use work-migration-verify-request/v1.")
    requirement = nonempty_string(request["requirement_id"], location="requirement_id")
    record_id = nonempty_string(request["record_id"], location="record_id")
    if not re.fullmatch(r"SPEC-UPDATE-\d{3}", record_id):
        _fail("migration_verify_record_id", "Use an exact SPEC-UPDATE-nnn record ID.")
    declared = request["artifacts"]
    artifacts = validate_artifact_paths(
        project_root, requirement, declared,
        actual_plan_path=declared.get("plan") if isinstance(declared, dict) else "",
    )
    execution = artifacts["execution"]
    relative = execution + "/.work-spec-update-" + record_id + ".json"
    paths = {
        "plan": artifacts["plan"], "task": artifacts["task"], "index": execution + "/index.json",
        "journal": relative, "completion": relative + ".done",
    }
    report = Diagnostics()
    report.check("writer_idle", lambda: require_idle_writer(project_root, execution))
    snapshots = {
        key: report.check("read:" + key, lambda path=path: read_raw(storage_path(project_root, path)), location=path)
        for key, path in paths.items()
    }
    fingerprints = {}
    for key in ("plan", "task", "index"):
        raw = snapshots[key]
        fingerprints[key] = {"raw_sha256": _hash(raw) if raw is not None else None}
        if raw is not None:
            fingerprints[key]["canonical_sha256"] = report.check(
                "fingerprint:" + key, lambda raw=raw, key=key: canonical_sha256(raw, source=key),
            )
        else:
            report.skip("fingerprint:" + key, "read:" + key)
    journal_raw = snapshots["journal"]
    record = None
    if journal_raw is not None:
        record = report.check("journal_contract", lambda: _record(journal_raw, record_id, requirement, artifacts))
    else:
        report.skip("journal_contract", "read:journal")
    if journal_raw is not None and snapshots["completion"] is not None:
        report.check("completion_marker", lambda: _same(
            snapshots["completion"], _hash(journal_raw).encode("ascii") + b"\n",
            "migration_verify_completion_mismatch", "The completion marker does not match the journal SHA-256.",
        ))
    else:
        report.skip("completion_marker", "read:journal", "read:completion")

    # Current-source validation is independent of the availability of a journal.
    diagnostics = diagnose_task_contract(
        snapshots["task"], source="migration verification TASK", actual_task_path=artifacts["task"],
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        plan_path=artifacts["plan"], execution_dir=execution,
        _source_plan_raw=snapshots["plan"], _index_raw=snapshots["index"],
    )
    transactions = _transactions(project_root, execution, report)
    from .migration_transactions import verification_selection
    selection = report.check("verification_selection", lambda: verification_selection(project_root, execution, snapshots))
    history = report.check("history_snapshot", lambda: _history_snapshot(project_root, execution))
    history_baseline = (
        _history_without_transaction(project_root, execution, record_id)
        if history is not None else None
    )
    history_validation = {}
    if history is not None:
        directories = {str(Path(path).parent.relative_to(Path(execution)).parts[0]) for path in history}
        for directory in sorted(directories):
            report.check("history_layout:" + directory, lambda directory=directory: validate_execution_task_layout(
                project_root, execution + "/" + directory,
            ))
        for path in sorted(history):
            if path.endswith("/attempt.json"):
                operation = lambda path=path: validate_attempt_file(project_root, path)
            elif "/corrections/" in path and path.endswith(".json"):
                operation = lambda path=path: validate_correction_file(project_root, path)
            else:
                continue
            history_validation[path] = report.check("history_contract:" + path, operation, location=path)
    else:
        report.skip("history_contracts", "history_snapshot")

    index = report.check("index_json", lambda: parse_json_contract(snapshots["index"], source="current index")) if snapshots["index"] is not None else None
    rows = index.get("tasks") if isinstance(index, dict) else None
    if isinstance(rows, list) and history is not None:
        for number, row in enumerate(rows):
            def references(row=row):
                if not isinstance(row, dict) or not isinstance(row.get("id"), str):
                    _fail("migration_verify_index_row", "An execution row has no valid TASK identity.")
                directory = execution + "/" + row["id"] + "/"
                if "latest_attempt" in row:
                    path = directory + str(row["latest_attempt"]) + "/attempt.json"
                    if path not in history or history_validation.get(path) is None:
                        _fail("migration_verify_attempt_reference", "The index references an unavailable or invalid Attempt.", path=path)
                if "latest_correction" in row:
                    correction = str(row["latest_correction"])
                    path = directory + correction.split("-CORRECTION-", 1)[0] + "/corrections/" + correction + ".json"
                    if path not in history or history_validation.get(path) is None:
                        _fail("migration_verify_correction_reference", "The index references an unavailable or invalid Correction.", path=path)
                return True
            report.check("index_history_binding:" + str(number), references)
    else:
        report.skip("index_history_bindings", "index_json", "history_snapshot")

    if record is not None:
        for key in ("plan", "task", "index"):
            if snapshots[key] is not None:
                report.check("installed:" + key, lambda key=key: _same(
                    snapshots[key], record["after"][key].encode("utf-8"),
                    "migration_verify_state_changed",
                    "The current artifact differs from this migration's installed result; inspect later activity or edits.",
                    artifact=key,
                ))
            else:
                report.skip("installed:" + key, "read:" + key)
        if history is not None:
            report.check("history_preserved", lambda: _same(
                history_baseline, record["history_sha256"], "migration_verify_history_changed",
                "Execution history differs from the migration snapshot; inspect later execution or edits.",
            ))
        else:
            report.skip("history_preserved", "history_snapshot")
        def reproduce():
            replay_request = copy.deepcopy(record["request"])
            for artifact in ("plan", "task"):
                installed = json.loads(record["after"][artifact])
                _same(
                    _json(replay_request[artifact]), _json(installed),
                    "migration_verify_record_mismatch",
                    "The approved request differs from the journal's installed candidate.",
                    artifact=artifact,
                )
                # Journal serialization sorts request keys. Preserve the saved
                # candidate's nested key order only after proving equal content.
                replay_request[artifact] = installed
            rebuilt, _ = _prepare(
                replay_request, project_root=project_root, user_config_root=user_config_root,
                skill_roots=skill_roots, before=record["before"], migration=True,
            )
            # Reuse only the read-only preparation path, never apply/recover.
            for key in ("before", "after"):
                for artifact in ("plan", "task", "index"):
                    if json.loads(rebuilt[key][artifact]) != json.loads(record[key][artifact]):
                        _fail(
                            "migration_verify_record_mismatch",
                            "The journal does not match revalidated migration content, index effects or current Execute guidance.",
                            artifact=artifact,
                        )
            _same(
                rebuilt["history_sha256"], record["history_sha256"],
                "migration_verify_record_mismatch",
                "The journal does not match revalidated migration content, index effects or current Execute guidance.",
            )
            _same(
                rebuilt["affected_task_ids"], record["affected_task_ids"],
                "migration_verify_record_mismatch",
                "The journal does not match revalidated migration content, index effects or current Execute guidance.",
            )
            _same(
                _json(rebuilt["migration"]), _json(record["migration"]),
                "migration_verify_record_mismatch",
                "The journal's migration evidence differs from the revalidated Plan edits, Execute guidance or binding repair.",
            )
            return True
        report.check("migration_evidence", reproduce)
    else:
        for name in ("installed_artifacts", "history_preserved", "migration_evidence"):
            report.skip(name, "journal_contract")

    git = _git_visibility(project_root, paths)
    # A report cannot certify a mixture of snapshots observed during a writer.
    for key, raw in snapshots.items():
        if raw is not None:
            report.check("unchanged:" + key, lambda key=key, raw=raw: _same(
                read_raw(storage_path(project_root, paths[key])), raw,
                "migration_verify_concurrent_change", "A verification source changed during inspection.", artifact=key,
            ))
    if history is not None:
        report.check("history_unchanged", lambda: _same(
            _history_without_transaction(project_root, execution, record_id), history_baseline,
            "migration_verify_concurrent_history",
            "Execution history changed during verification.",
        ))
    report.check("writer_idle_after", lambda: require_idle_writer(project_root, execution))
    verified = (
        diagnostics["normal_use_allowed"] and diagnostics["repair_mode"] == "review_required"
        and all(check["status"] == "passed" for check in report.checks)
        and all(item["completion"] == "completed" for item in transactions)
    )
    return {
        "schema": "work-migration-verification/v1", "status": "verified" if verified else "blocked",
        "verified": verified, "record_id": record_id, "requirement_id": requirement, "artifacts": artifacts,
        "verification_scope": "exact_migration_result", "journal_sha256": _hash(journal_raw) if journal_raw is not None else None,
        "completion_status": report.status("completion_marker"), "fingerprints": fingerprints,
        "checks": report.checks, "issues": report.issues, "task_diagnostics": diagnostics,
        "execution_tasks": rows, "history_sha256": history, "history_validation": history_validation,
        "transactions": transactions, "git": git,
        "verification_selection": selection,
        "execution_authorized": False,
        "next_step": "normal_execute_preflight" if verified else "review_failed_and_not_checked_checks",
    }
