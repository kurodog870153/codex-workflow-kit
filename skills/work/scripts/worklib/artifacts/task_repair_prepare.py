"""Assemble reviewed TASK repairs without guessing missing or ambiguous values."""
from __future__ import annotations

import copy
from pathlib import Path

from .task_repair import _fail, _json, repair_task
from ..contracts.task_diagnostics import _json_document, diagnose_task_contract
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.errors import WorkError
from ..foundation.fingerprint import decode_utf8, raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import validate_artifact_paths
from ..foundation.spec_update import require_idle_writer, require_no_spec_update, storage_path


def _apply_edits(task, edits):
    if not isinstance(edits, list):
        _fail("task_repair_prepare_edits", "Supply an array of confirmed field edits.")
    seen = set()
    for edit in edits:
        edit = strict_keys(edit, location="repair_edit", required={"operation", "field"},
                           optional={"task_id", "before", "after"})
        operation = edit["operation"]
        if operation not in ("add", "replace", "remove"):
            _fail("task_repair_prepare_operation", "Use add, replace or remove.")
        required = {"operation", "field"}
        if operation != "add":
            required.add("before")
        if operation != "remove":
            required.add("after")
        strict_keys(edit, location="repair_edit", required=required, optional={"task_id"})
        field = nonempty_string(edit["field"], location="repair_edit.field")
        task_id = edit.get("task_id")
        if "task_id" in edit:
            nonempty_string(task_id, location="repair_edit.task_id")
        identity = (task_id, field)
        if identity in seen or (task_id is not None and (None, "tasks") in seen) or (
            identity == (None, "tasks") and any(key[0] is not None for key in seen)
        ):
            _fail("task_repair_prepare_overlap", "Repair each field once; do not overlap TASK rows and their array.")
        seen.add(identity)
        target = task
        if task_id is not None:
            rows = task.get("tasks")
            matches = [row for row in rows if isinstance(row, dict) and row.get("id") == task_id] if isinstance(rows, list) else []
            if len(matches) != 1:
                _fail("task_repair_prepare_task_id", "A row edit requires one unambiguous existing TASK ID.")
            target = matches[0]
        if operation == "add":
            if field in target:
                _fail("task_repair_prepare_existing_field", "Add requires an absent field, including absence of null.")
        elif field not in target or _json(target[field]) != _json(edit["before"]):
            _fail("task_repair_prepare_old_value", "The confirmed original field value changed or is missing.")
        if operation == "remove":
            del target[field]
        else:
            if operation == "replace" and _json(edit["before"]) == _json(edit["after"]):
                _fail("task_repair_prepare_unchanged", "A replacement must change its field.")
            target[field] = copy.deepcopy(edit["after"])
    return task


def prepare_task_repair(raw_request, *, project_root: Path, user_config_root: str,
                        skill_roots=None, output_file: str | None = None):
    request = strict_keys(parse_json_contract(raw_request, source="TASK repair preparation"),
        location="task_repair_prepare", required={"schema", "stage", "requirement_id", "artifacts", "decisions"},
        optional={"task", "edits"})
    if request["schema"] != "work-task-repair-prepare-request/v1" or request["stage"] not in ("format", "complete"):
        _fail("task_repair_prepare_schema", "Use work-task-repair-prepare-request/v1 and stage format or complete.")
    if "task" in request and "edits" in request:
        _fail("task_repair_prepare_candidate", "Supply field edits or an explicit candidate, not both.")
    requirement = nonempty_string(request["requirement_id"], location="requirement_id")
    declared = request["artifacts"]
    artifacts = validate_artifact_paths(project_root, requirement, declared,
        actual_plan_path=declared.get("plan") if isinstance(declared, dict) else "")
    if artifacts != declared:
        _fail("task_repair_paths", "Repair requests must use normalized project-relative paths.")
    require_idle_writer(project_root, artifacts["execution"])
    require_no_spec_update(project_root, artifacts["execution"])
    paths = {key: storage_path(project_root, artifacts[key]) for key in ("plan", "task")}
    paths["index"] = storage_path(project_root, artifacts["execution"] + "/index.json")
    before = {key: read_raw(path) for key, path in paths.items()}
    options = dict(project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    if "task" in request:
        task = copy.deepcopy(request["task"])
    else:
        try:
            task = _json_document(decode_utf8(before["task"], source="original TASK"), before["task"])
        except WorkError as error:
            error.details["task_diagnostics"] = diagnose_task_contract(before["task"],
                source="original TASK", actual_task_path=artifacts["task"], plan_path=artifacts["plan"],
                execution_dir=artifacts["execution"], _source_plan_raw=before["plan"], _index_raw=before["index"],
                **options)
            raise
        task = _apply_edits(task, request.get("edits", []))
    prepared = {"schema": "work-task-repair-request/v1", "stage": request["stage"],
        "requirement_id": requirement, "artifacts": artifacts, "decisions": request["decisions"],
        "expected": {key + "_sha256": raw_sha256(raw) for key, raw in before.items()}, "task": task}
    # Use identical nested key ordering for validation and subsequent transport.
    prepared = parse_json_contract(_json(prepared), source="prepared TASK repair")
    preview = repair_task(_json(prepared), **options)
    if any(read_raw(path) != before[key] for key, path in paths.items()):
        _fail("task_repair_source_changed", "Original artifacts changed during repair preparation.")
    require_idle_writer(project_root, artifacts["execution"])
    require_no_spec_update(project_root, artifacts["execution"])
    if output_file is not None:
        with Path(output_file).open("xb") as stream:
            stream.write(_json(prepared))
    return {"schema": "work-task-repair-prepare/v1", "request": prepared,
            "preview": preview, "output_file": output_file}
