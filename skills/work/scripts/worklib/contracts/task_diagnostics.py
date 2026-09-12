"""Read-only TASK diagnostics. A report never authorizes repair or execution."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Callable

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, canonical_text, decode_utf8, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path, validate_artifact_paths
from ..foundation.runtime import installed_work_root
from ..foundation.spec_update import storage_path
from ..instructions.historical import stored_document_selection
from ..instructions.selection import validate_instruction_selection
from ..instructions.task_selection import validate_task_document_instruction_selection
from .execution_index import validate_execution_index
from .plan import validate_plan_contract
from .task_structure import inspect_task_structure


class Diagnostics:
    def __init__(self):
        self.checks: list[dict[str, Any]] = []
        self.issues: list[dict[str, Any]] = []

    def skip(self, name: str, *dependencies: str):
        self.checks.append({"name": name, "status": "not_checked", "requires": list(dependencies)})
        return None

    def check(self, name: str, operation: Callable, *, location: str = ""):
        try:
            value = operation()
        except WorkError as error:
            self.failure(name, error, location=location)
            return None
        except (TypeError, KeyError, ValueError, RecursionError) as error:
            self.skip(name, "supported_input_shape")
            self.issues.append({
                "stage": name, "code": "diagnostic_input_not_supported", "location": location,
                "category": "review_required",
                "message": "This check cannot continue with the supplied value.",
                "suggestion": "Review the input shape, then repeat diagnosis.",
                "details": {"exception_type": type(error).__name__},
            })
            return None
        self.checks.append({"name": name, "status": "passed"})
        return value

    def failure(self, name: str, error: WorkError, *, location: str = ""):
        self.checks.append({"name": name, "status": "failed"})
        category = "review_required"
        suggestion = "Review the evidence before preparing a repair; do not change source fingerprints alone."
        if name in {"encoding", "json"}:
            category = "user_decision"
            suggestion = "Preserve the original bytes; ask the user to resolve any ambiguous decoding or JSON interpretation."
        elif name in {"normalization", "canonical"}:
            category = "format_repair"
            suggestion = "Preview a lossless canonical UTF-8 rendering before requesting write approval."
        elif name.startswith("instructions"):
            category = "migration_review"
        self.issues.append({
            "stage": name, "code": error.code, "location": location,
            "category": category, "message": error.message,
            "suggestion": suggestion, "details": dict(error.details),
        })

    def passed(self, name: str) -> bool:
        return any(check["name"] == name and check["status"] == "passed" for check in self.checks)

    def status(self, name: str) -> str:
        return next((check["status"] for check in self.checks if check["name"] == name), "not_checked")


def _reject(code: str, message: str, **details):
    raise WorkError(ExitCode.CONTRACT, code, message, details)


def _json_document(text: str, raw: bytes):
    duplicates: list[str] = []

    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                duplicates.append(key)
            result[key] = value
        return result

    def constant(value):
        _reject("invalid_json_constant", "JSON cannot contain non-standard numeric constants.")

    try:
        value = json.loads(text, object_pairs_hook=object_pairs, parse_constant=constant)
    except json.JSONDecodeError as error:
        bom_size = 3 if raw.startswith(b"\xef\xbb\xbf") else 0
        _reject("invalid_json_contract", "The TASK JSON is invalid.",
                line=error.lineno, column=error.colno,
                byte_offset=bom_size + len(text[:error.pos].encode("utf-8")))
    except (RecursionError, ValueError):
        _reject("invalid_json_contract", "The TASK JSON exceeds the parser's supported limits.")
    # Escaped unpaired surrogates cannot be rendered as UTF-8.
    try:
        json.dumps(value, ensure_ascii=False).encode("utf-8")
    except UnicodeEncodeError:
        _reject("invalid_json_unicode", "JSON strings contain an unpaired Unicode surrogate.")
    if duplicates:
        _reject("duplicate_json_key", "Duplicate keys are ambiguous; no parsed document will be used.",
                keys=sorted(set(duplicates)))
    if not isinstance(value, dict):
        _reject("json_contract_not_object", "The TASK document must be a JSON object.")
    return value


def _normalization(text, raw):
    if canonical_text(text).encode("utf-8") != raw:
        _reject("noncanonical_task_text", "TASK text must be UTF-8 without BOM, NFC, LF and one trailing LF.")
    return True


def _same(actual, expected, code, message):
    if actual != expected:
        _reject(code, message, expected=expected, actual=actual)
    return True


def _execution_state(report, project_root, execution_dir, *, index_raw=None, ignored_record=None):
    """Inspect evidence without opening or creating a writer mutex."""
    if execution_dir is None:
        report.skip("index", "artifact_paths")
        report.skip("repair_state", "artifact_paths")
        return None
    directory = report.check("execution_path", lambda: storage_path(project_root, execution_dir))
    if directory is None:
        report.skip("index", "execution_path")
        report.skip("repair_state", "execution_path")
        return None

    def index():
        path = storage_path(project_root, execution_dir + "/index.json")
        raw = index_raw if index_raw is not None else read_raw(path)
        validate_execution_index(raw, source=str(path))
        return parse_json_contract(raw, source=str(path))

    value = report.check("index", index, location=execution_dir + "/index.json")
    before = len(report.issues)
    if value is None:
        report.skip("repair_state", "index")
    else:
        if "lock" in value:
            report.failure("execution_lock", WorkError(
                ExitCode.LOCK_CONFLICT, "task_repair_execution_lock",
                "An execution lock permits diagnosis only.",
            ))
        if any(row["status"] == "in_progress" for row in value["tasks"]):
            report.failure("active_task", WorkError(
                ExitCode.LOCK_CONFLICT, "task_repair_active_task",
                "An in-progress TASK permits diagnosis only.",
            ))

    # Pending records and active Attempts are independent of TASK parseability.
    try:
        pending = sorted(path.relative_to(project_root).as_posix() for path in directory.rglob(".work-*.tmp"))
        records = sorted([*directory.glob(".work-spec-update-*.json"), *directory.glob(".work-task-repair-*.json")])
        for record in records:
            if record.relative_to(project_root).as_posix() == ignored_record:
                continue
            record = storage_path(project_root, record.relative_to(project_root).as_posix())
            marker = storage_path(project_root, record.relative_to(project_root).as_posix() + ".done")
            expected = hashlib.sha256(read_raw(record)).hexdigest().encode("ascii") + b"\n"
            if not marker.is_file() or read_raw(marker) != expected:
                pending.append(record.relative_to(project_root).as_posix())
        if pending:
            _reject("task_repair_pending_transaction", "An incomplete transaction permits diagnosis only.",
                    paths=pending)
        report.checks.append({"name": "transactions", "status": "passed"})
    except WorkError as error:
        report.failure("transactions", error)
    except OSError:
        report.failure("transactions", WorkError(
            ExitCode.IO_FAILURE, "task_transaction_scan_failed",
            "Transaction evidence could not be inspected.",
        ))

    from .attempt import validate_attempt_json_contract
    for path in sorted(directory.glob("TASK-*/ATTEMPT-*/attempt.json")):
        relative = path.relative_to(project_root).as_posix()

        def attempt(path=path, relative=relative):
            checked = storage_path(project_root, relative)
            raw = read_raw(checked)
            validate_attempt_json_contract(raw, source=str(path), project_root=project_root)
            contract = parse_json_contract(raw, source=str(path))
            if contract["status"] == "in_progress":
                _reject("task_repair_active_attempt", "An active Attempt permits diagnosis only.", path=relative)
            return True

        report.check("attempt:" + relative, attempt, location=relative)
    if value is not None:
        report.checks.append({
            "name": "repair_state",
            "status": "passed" if len(report.issues) == before else "failed",
        })
    return value


def diagnose_task_contract(
    raw: bytes | None, *, source: str, actual_task_path: str,
    project_root: Path, user_config_root: str, skill_roots=None,
    plan_path: str | None = None, execution_dir: str | None = None,
    validate_file_state: bool = False, _source_plan_raw: bytes | None = None,
    _historical_work_sources: bool = False, _reviewed_source_plan_binding=None,
    _contract_error: WorkError | None = None, _read_error: WorkError | None = None,
    _index_raw: bytes | None = None, _ignored_repair_record: str | None = None,
) -> dict[str, Any]:
    from .task import _validate_task_contract, render_task_contract
    from .execution_index import build_initial_execution_index

    report = Diagnostics()
    if _read_error is not None:
        report.failure("file", _read_error, location=actual_task_path)
    else:
        report.checks.append({"name": "file", "status": "passed"})
    text = None
    if raw is not None:
        text = report.check("encoding", lambda: decode_utf8(raw, source=source))
    else:
        report.skip("encoding", "file")
    document = None
    if text is not None:
        document = report.check("json", lambda: _json_document(text, raw))
        report.check("normalization", lambda: _normalization(text, raw))
    else:
        report.skip("json", "encoding")
        report.skip("normalization", "encoding")

    if document is not None:
        issues = inspect_task_structure(document)
        report.issues.extend(issues)
        report.checks.append({"name": "structure", "status": "failed" if issues else "passed"})

        def artifact_paths():
            if not isinstance(document.get("requirement_id"), str):
                _reject("invalid_requirement_id", "The requirement ID must be a string.")
            return validate_artifact_paths(
                project_root, document.get("requirement_id"), document.get("artifacts"),
                actual_plan_path=plan_path or (
                    document["artifacts"].get("plan") if isinstance(document.get("artifacts"), dict) else ""
                ),
            )

        artifacts = report.check("artifact_paths", artifact_paths, location="/artifacts")
        if artifacts is not None:
            report.check("task_path_binding", lambda: _same(
                artifacts["task"], actual_task_path, "task_artifact_path_mismatch",
                "The TASK artifact path differs from the selected file.",
            ), location="/artifacts/task")
            if plan_path is None:
                plan_path = artifacts["plan"]
            if execution_dir is None:
                execution_dir = artifacts["execution"]
            else:
                report.check("execution_path_binding", lambda: _same(
                    artifacts["execution"], execution_dir, "task_execution_path_mismatch",
                    "The selected execution directory differs from the TASK.",
                ), location="/artifacts/execution")
        if report.passed("structure"):
            report.check("canonical", lambda: _same(
                raw == render_task_contract(document), True, "noncanonical_json_contract",
                "The stored TASK does not match canonical field order and serialization.",
            ))
        else:
            report.skip("canonical", "structure")
    else:
        report.skip("structure", "json")
        report.skip("artifact_paths", "json")
        report.skip("canonical", "structure")

    def plan():
        normalized, path = resolve_project_relative_path(project_root, plan_path, field="plan_path")
        plan_raw = _source_plan_raw if _source_plan_raw is not None else read_raw(path)
        return validate_plan_contract(
            plan_raw, source="TASK diagnostic Plan", actual_plan_path=normalized,
            project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
            _historical_work_sources=_historical_work_sources,
        )

    plan_result = report.check("plan", plan, location=plan_path) if plan_path is not None else report.skip("plan", "artifact_paths")
    if plan_result is not None and document is not None and isinstance(document.get("source_plan"), dict):
        for field, expected in (
            ("canonical_sha256", plan_result["plan_sha256"]),
            ("hierarchy_selection_sha256", plan_result["hierarchy_selection_sha256"]),
        ):
            report.check("plan_binding:" + field, lambda field=field, expected=expected: _same(
                document["source_plan"].get(field), expected, "source_plan_fingerprint_mismatch",
                "The TASK source binding differs from the validated Plan.",
            ), location="/source_plan/" + field)
    else:
        report.skip("plan_binding", "plan", "source_plan")

    if document is not None and isinstance(document.get("tasks"), list):
        tasks = document["tasks"]
        selections = []
        valid_selections = True
        for index, task in enumerate(tasks):
            if not isinstance(task, dict) or "instruction_selection" not in task:
                report.skip("instructions:" + str(index), "structure")
                valid_selections = False
                continue
            selection = task["instruction_selection"]
            selections.append(selection)
            if _historical_work_sources:
                from ..instructions.historical import stored_selection
                operation = lambda selection=selection: stored_selection(selection)
            else:
                operation = lambda selection=selection, index=index: validate_instruction_selection(
                    selection, skill_root=installed_work_root(), mode="task",
                    location=f"tasks[{index}].instruction_selection",
                )
            value = report.check("instructions:" + str(index), operation,
                                 location=f"/tasks/{index}/instruction_selection")
            valid_selections &= value is not None
        if valid_selections and selections and isinstance(document.get("instruction_selection"), dict):
            operation = (
                lambda: stored_document_selection(document["instruction_selection"], selections)
            ) if _historical_work_sources else (
                lambda: validate_task_document_instruction_selection(
                    document["instruction_selection"], selections, skill_root=installed_work_root(),
                )
            )
            report.check("instructions_union", operation, location="/instruction_selection")
        else:
            report.skip("instructions_union", "instructions", "structure")
    else:
        report.skip("instructions", "structure")

    contract_result = None
    if _contract_error is not None:
        report.failure("contract", _contract_error)
    elif raw is not None and report.passed("structure"):
        contract_result = report.check("contract", lambda: _validate_task_contract(
            raw, source=source, actual_task_path=actual_task_path,
            project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
            validate_file_state=validate_file_state, _source_plan_raw=_source_plan_raw,
            _historical_work_sources=_historical_work_sources,
            _reviewed_source_plan_binding=_reviewed_source_plan_binding,
        ))
    else:
        report.skip("contract", "structure")
    if not report.passed("contract"):
        report.skip("remaining_contract_checks", "contract")

    index = _execution_state(report, project_root, execution_dir,
                             index_raw=_index_raw, ignored_record=_ignored_repair_record)
    if index is not None and document is not None and text is not None:
        bindings = {
            "task_sha256": canonical_sha256(raw, source=source),
            "task_spec_id": document.get("spec_id"),
            "requirement_id": document.get("requirement_id"),
        }
        for field, expected in bindings.items():
            report.check("index_binding:" + field, lambda field=field, expected=expected: _same(
                index[field], expected, "task_index_binding_mismatch",
                "The execution index differs from the TASK document.",
            ), location="/index/" + field)
    else:
        report.skip("index_binding", "index", "json")

    if index is not None and contract_result is not None:
        expected_index = build_initial_execution_index(document, contract_result)
        for field in ("task_instructions_sha256", "hierarchy_selection_sha256", "skill_selection_sha256"):
            report.check("index_binding:" + field, lambda field=field: _same(
                index[field], expected_index[field], "task_index_binding_mismatch",
                "The execution index source binding differs from the TASK.",
            ), location="/index/" + field)
        report.check("index_binding:tasks", lambda: _same(
            [{key: row[key] for key in ("id", "skill_id", "instructions_sha256")} for row in index["tasks"]],
            [{key: row[key] for key in ("id", "skill_id", "instructions_sha256")} for row in expected_index["tasks"]],
            "task_index_rows_mismatch", "Execution rows differ from the validated TASK identities.",
        ), location="/index/tasks")
    else:
        report.skip("index_binding:sources_and_rows", "index", "contract")
    required_bindings = [
        check for check in report.checks
        if "binding" in check["name"] or check["name"] == "artifact_paths"
    ]
    allowed = (report.passed("contract") and report.passed("transactions")
               and all(check["status"] == "passed" for check in required_bindings))
    return {
        "schema": "work-task-diagnostics/v1",
        "status": "valid" if allowed else "blocked",
        "task_path": actual_task_path,
        "raw_sha256": hashlib.sha256(raw).hexdigest() if raw is not None else None,
        "format_status": (
            "passed" if all(report.passed(name) for name in ("encoding", "json", "normalization"))
            else "failed" if any(report.status(name) == "failed" for name in ("encoding", "json", "normalization"))
            else "not_checked"
        ),
        "structure_status": report.status("structure"),
        "contract_status": report.status("contract"),
        "normal_use_allowed": allowed,
        "repair_mode": "review_required" if report.passed("repair_state") else "diagnose_only",
        "checks": report.checks,
        "issues": report.issues,
    }


def diagnose_task_file(
    project_root: Path, user_config_root: str, raw_path: str, *,
    plan_path: str | None = None, execution_dir: str | None = None, skill_roots=None,
) -> dict[str, Any]:
    raw, error = None, None
    normalized, path = resolve_project_relative_path(project_root, raw_path, field="task_path")
    try:
        raw = read_raw(path)
    except WorkError as caught:
        error = caught
    return diagnose_task_contract(
        raw, source=str(path), actual_task_path=normalized, project_root=project_root,
        user_config_root=user_config_root, plan_path=plan_path, execution_dir=execution_dir,
        skill_roots=skill_roots, _read_error=error,
    )
