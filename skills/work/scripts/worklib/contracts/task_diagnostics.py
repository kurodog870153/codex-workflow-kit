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
from ..services.instruction_history import stored_document_selection
from ..services.instruction_selection import validate_instruction_selection
from ..services.instruction_task_selection import validate_task_document_instruction_selection
from .execution_index import build_initial_execution_index, validate_execution_index
from ..services.plan_validation import validate_plan_contract
from .task_structure import inspect_task_structure
from .task_collection_models import TaskCollectionDiagnosticsContract


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
        leaf = name.rsplit(":", 1)[-1]
        if leaf in {"encoding", "json"}:
            category = "user_decision"
            suggestion = "Preserve the original bytes; ask the user to resolve any ambiguous decoding or JSON interpretation."
        elif leaf in {"normalization", "canonical"}:
            category = "format_repair"
            suggestion = "Preview a lossless canonical UTF-8 rendering before requesting write approval."
        elif name.startswith("instructions"):
            category = "source_review"
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


def diagnose_task_collection(
    project_root: Path,
    user_config_root: str,
    raw_index_path: str,
    *,
    skill_roots=None,
) -> dict[str, Any]:
    """Diagnose a TASK collection without repairing or publishing any artifact."""
    from .task_collection import validate_task_collection_contract
    from .task_index import render_task_index_contract, validate_task_index_contract
    from .task_item import render_task_item_contract, validate_task_item_contract
    from ..foundation.paths import resolve_task_collection_item_path

    report = Diagnostics()
    resolved = report.check(
        "index:path",
        lambda: resolve_project_relative_path(
            project_root, raw_index_path, field="task_index_path"
        ),
        location=raw_index_path,
    )
    if resolved is None:
        report.skip("index:file", "index:path")
        index_raw = None
    else:
        normalized_index, index_path = resolved
        index_raw = report.check(
            "index:file", lambda: read_raw(index_path), location=normalized_index
        )

    index_text = None
    if index_raw is not None:
        index_text = report.check(
            "index:encoding",
            lambda: decode_utf8(index_raw, source=raw_index_path),
            location=raw_index_path,
        )
    else:
        report.skip("index:encoding", "index:file")
    index_document = None
    if index_text is not None:
        index_document = report.check(
            "index:json",
            lambda: _json_document(index_text, index_raw),
            location=raw_index_path,
        )
        report.check(
            "index:normalization",
            lambda: _normalization(index_text, index_raw),
            location=raw_index_path,
        )
    else:
        report.skip("index:json", "index:encoding")
        report.skip("index:normalization", "index:encoding")

    if index_document is not None:
        report.check(
            "index:schema",
            lambda: _same(
                index_document.get("schema"),
                "work-task-index/v1",
                "invalid_task_index_schema",
                "The formal TASK index schema is invalid.",
            ),
            location="/schema",
        )
        report.check(
            "index:canonical",
            lambda: _same(
                index_raw,
                render_task_index_contract(index_document),
                "noncanonical_json_contract",
                "The TASK index does not match canonical field order and serialization.",
            ),
            location=raw_index_path,
        )
        index_validation = report.check(
            "index:contract",
            lambda: validate_task_index_contract(
                index_raw,
                source=raw_index_path,
                actual_index_path=raw_index_path,
                project_root=project_root,
            ),
            location=raw_index_path,
        )
    else:
        report.skip("index:schema", "index:json")
        report.skip("index:canonical", "index:json")
        index_validation = report.skip("index:contract", "index:json")

    raw_items: dict[str, bytes] = {}
    item_documents: dict[str, dict[str, Any]] = {}
    references = index_document.get("tasks") if isinstance(index_document, dict) else None
    requirement_id = index_document.get("requirement_id") if isinstance(index_document, dict) else None
    if not isinstance(references, list) or not isinstance(requirement_id, str):
        report.skip("items", "index:json", "index:structure")
    else:
        for position, reference in enumerate(references):
            task_id = reference.get("id") if isinstance(reference, dict) else None
            label = task_id if isinstance(task_id, str) else str(position)
            prefix = "item:" + label
            if not isinstance(reference, dict) or not isinstance(task_id, str) or not isinstance(reference.get("path"), str):
                report.skip(prefix + ":path", "index:structure")
                report.skip(prefix + ":file", prefix + ":path")
                report.skip(prefix + ":contract", prefix + ":file")
                continue
            item_path_result = report.check(
                prefix + ":path",
                lambda reference=reference, task_id=task_id: resolve_task_collection_item_path(
                    project_root,
                    requirement_id,
                    raw_index_path,
                    task_id,
                    reference["path"],
                ),
                location=f"/tasks/{position}/path",
            )
            if item_path_result is None:
                report.skip(prefix + ":file", prefix + ":path")
                report.skip(prefix + ":encoding", prefix + ":file")
                report.skip(prefix + ":json", prefix + ":encoding")
                report.skip(prefix + ":contract", prefix + ":json")
                continue
            _, item_path = item_path_result
            raw = report.check(
                prefix + ":file",
                lambda item_path=item_path: read_raw(item_path),
                location=str(item_path),
            )
            if raw is None:
                report.skip(prefix + ":encoding", prefix + ":file")
                report.skip(prefix + ":json", prefix + ":encoding")
                report.skip(prefix + ":contract", prefix + ":json")
                report.skip(prefix + ":fingerprint", prefix + ":contract")
                continue
            raw_items[task_id] = raw
            text = report.check(
                prefix + ":encoding",
                lambda raw=raw, item_path=item_path: decode_utf8(raw, source=str(item_path)),
                location=str(item_path),
            )
            if text is None:
                report.skip(prefix + ":json", prefix + ":encoding")
                report.skip(prefix + ":normalization", prefix + ":encoding")
                report.skip(prefix + ":contract", prefix + ":json")
                report.skip(prefix + ":fingerprint", prefix + ":contract")
                continue
            document = report.check(
                prefix + ":json",
                lambda text=text, raw=raw: _json_document(text, raw),
                location=str(item_path),
            )
            report.check(
                prefix + ":normalization",
                lambda text=text, raw=raw: _normalization(text, raw),
                location=str(item_path),
            )
            if document is None:
                report.skip(prefix + ":canonical", prefix + ":json")
                report.skip(prefix + ":contract", prefix + ":json")
                report.skip(prefix + ":fingerprint", prefix + ":contract")
                continue
            item_documents[task_id] = document
            report.check(
                prefix + ":canonical",
                lambda document=document, raw=raw: _same(
                    raw,
                    render_task_item_contract(document),
                    "noncanonical_json_contract",
                    "The TASK item does not match canonical field order and serialization.",
                ),
                location=str(item_path),
            )
            item_validation = report.check(
                prefix + ":contract",
                lambda raw=raw, task_id=task_id, item_path=item_path: validate_task_item_contract(
                    raw, source=str(item_path), expected_task_id=task_id
                ),
                location=str(item_path),
            )
            if item_validation is None:
                report.skip(prefix + ":fingerprint", prefix + ":contract")
            else:
                report.check(
                    prefix + ":fingerprint",
                    lambda item_validation=item_validation, reference=reference: _same(
                        item_validation["task_item_sha256"],
                        reference.get("canonical_sha256"),
                        "task_item_fingerprint_mismatch",
                        "The TASK item fingerprint differs from the formal index.",
                    ),
                    location=f"/tasks/{position}/canonical_sha256",
                )

    directory_ready = resolved is not None and isinstance(references, list)
    if directory_ready:
        _, index_path = resolved
        item_directory = index_path.parent / "tasks"
        expected_names = {
            reference["path"].split("/", 1)[1]
            for reference in references
            if isinstance(reference, dict)
            and isinstance(reference.get("path"), str)
            and reference["path"].startswith("tasks/")
            and "/" not in reference["path"][6:]
        }

        def directory_check():
            try:
                observed = {
                    path.name
                    for path in item_directory.iterdir()
                    if path.suffix == ".json" and path.is_file()
                } if item_directory.is_dir() else set()
            except OSError as error:
                raise WorkError(
                    ExitCode.IO_FAILURE,
                    "task_collection_directory_read_failed",
                    "The TASK item directory could not be inspected.",
                    {"path": str(item_directory)},
                ) from error
            return _same(
                sorted(observed),
                sorted(expected_names),
                "task_collection_directory_mismatch",
                "The TASK item directory does not exactly match the formal index.",
            )

        report.check("items:directory", directory_check, location=str(item_directory))
    else:
        report.skip("items:directory", "index:path", "index:json")

    plan_raw = None
    artifacts = index_document.get("artifacts") if isinstance(index_document, dict) else None
    plan_path = artifacts.get("plan") if isinstance(artifacts, dict) else None
    if isinstance(plan_path, str):
        plan_resolved = report.check(
            "plan:path",
            lambda: resolve_project_relative_path(project_root, plan_path, field="plan_path"),
            location=plan_path,
        )
        if plan_resolved is not None:
            _, resolved_plan_path = plan_resolved
            plan_raw = report.check(
                "plan:file", lambda: read_raw(resolved_plan_path), location=plan_path
            )
        else:
            report.skip("plan:file", "plan:path")
        if plan_raw is not None:
            plan_validation = report.check(
                "plan:contract",
                lambda: validate_plan_contract(
                    plan_raw,
                    source=str(resolved_plan_path),
                    actual_plan_path=plan_path,
                    project_root=project_root,
                    user_config_root=user_config_root,
                    skill_roots=skill_roots,
                    _allow_task_index=True,
                ),
                location=plan_path,
            )
        else:
            plan_validation = report.skip("plan:contract", "plan:file")
        source_plan = index_document.get("source_plan")
        if plan_validation is not None and isinstance(source_plan, dict):
            report.check(
                "plan:binding:canonical_sha256",
                lambda: _same(
                    source_plan.get("canonical_sha256"),
                    canonical_sha256(plan_raw, source=str(resolved_plan_path)),
                    "source_plan_fingerprint_mismatch",
                    "The TASK collection source Plan fingerprint does not match the Plan.",
                ),
                location="/source_plan/canonical_sha256",
            )
            report.check(
                "plan:binding:hierarchy_selection_sha256",
                lambda: _same(
                    source_plan.get("hierarchy_selection_sha256"),
                    plan_validation["hierarchy_selection_sha256"],
                    "source_plan_hierarchy_selection_mismatch",
                    "The TASK collection hierarchy selection fingerprint does not match the Plan.",
                ),
                location="/source_plan/hierarchy_selection_sha256",
            )
        else:
            report.skip("plan:binding", "plan:contract", "index:source_plan")
    else:
        report.skip("plan:path", "index:artifacts")
        report.skip("plan:file", "plan:path")
        report.skip("plan:contract", "plan:file")
        report.skip("plan:binding", "plan:contract")

    item_contract_checks = [
        check for check in report.checks
        if check["name"].startswith("item:") and check["name"].endswith(":contract")
    ]
    prerequisites = (
        index_validation is not None
        and report.passed("items:directory")
        and plan_raw is not None
        and len(raw_items) == len(references or [])
        and item_contract_checks
        and all(check["status"] == "passed" for check in item_contract_checks)
    )
    if prerequisites:
        collection_validation = report.check(
            "collection:contract",
            lambda: validate_task_collection_contract(
                index_raw,
                raw_items,
                source=raw_index_path,
                actual_index_path=raw_index_path,
                project_root=project_root,
                user_config_root=user_config_root,
                validate_file_state=False,
                skill_roots=skill_roots,
                _source_plan_raw=plan_raw,
            ),
            location=raw_index_path,
        )
    else:
        collection_validation = report.skip(
            "collection:contract",
            "index:contract",
            "items:contracts",
            "items:directory",
            "plan:contract",
        )
    if collection_validation is not None:
        execution = index_document["artifacts"]["execution"] + "/index.json"
        execution_raw = report.check(
            "execution:file", lambda: read_raw(storage_path(project_root, execution)), location=execution,
        )
        if execution_raw is not None:
            execution_validation = report.check(
                "execution:contract",
                lambda: validate_execution_index(execution_raw, source=execution),
                location=execution,
            )
        else:
            execution_validation = report.skip("execution:contract", "execution:file")
        if execution_validation is not None:
            execution_contract = parse_json_contract(execution_raw, source=execution)
            expected = build_initial_execution_index(
                collection_validation["collection_contract"], collection_validation,
            )
            expected_binding = {
                "task_spec_id": expected["task_spec_id"],
                "task_collection_sha256": expected["task_collection_sha256"],
                "task_index_sha256": expected["task_index_sha256"],
                "task_instructions_sha256": expected["task_instructions_sha256"],
                "hierarchy_selection_sha256": expected["hierarchy_selection_sha256"],
                "skill_selection_sha256": expected["skill_selection_sha256"],
                "task_item_sha256": {row["id"]: row["task_item_sha256"] for row in expected["tasks"]},
            }
            actual_binding = {
                "task_spec_id": execution_contract["task_spec_id"],
                "task_collection_sha256": execution_contract["task_collection_sha256"],
                "task_index_sha256": execution_contract["task_index_sha256"],
                "task_instructions_sha256": execution_contract["task_instructions_sha256"],
                "hierarchy_selection_sha256": execution_contract["hierarchy_selection_sha256"],
                "skill_selection_sha256": execution_contract["skill_selection_sha256"],
                "task_item_sha256": {row["id"]: row["task_item_sha256"] for row in execution_contract["tasks"]},
            }
            report.check(
                "execution:binding",
                lambda: _same(actual_binding, expected_binding, "execution_task_binding_mismatch", "The execution index does not match the TASK collection."),
                location=execution,
            )
        else:
            report.skip("execution:binding", "execution:contract")
    else:
        report.skip("execution:file", "collection:contract")
        report.skip("execution:contract", "execution:file")
        report.skip("execution:binding", "execution:contract")
    report.skip("execution_index_binding", "collection_execution_index_contract")
    allowed = collection_validation is not None
    return TaskCollectionDiagnosticsContract.model_validate({
        "schema": "work-task-collection-diagnostics/v1",
        "status": "valid" if allowed else "blocked",
        "task_path": raw_index_path,
        "raw_sha256": hashlib.sha256(index_raw).hexdigest() if index_raw is not None else None,
        "format_status": (
            "passed"
            if all(report.passed(name) for name in ("index:encoding", "index:json", "index:normalization"))
            else "failed"
            if any(report.status(name) == "failed" for name in ("index:encoding", "index:json", "index:normalization"))
            else "not_checked"
        ),
        "contract_status": report.status("collection:contract"),
        "normal_use_allowed": allowed,
        "execution_binding_status": report.status("execution:binding"),
        "repair_mode": "review_required",
        "checks": report.checks,
        "issues": report.issues,
    }).to_canonical_dict()
