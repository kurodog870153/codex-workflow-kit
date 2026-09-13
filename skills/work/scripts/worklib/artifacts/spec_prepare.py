"""Assemble reviewed field replacements without publishing formal artifacts."""
from __future__ import annotations

import copy
from datetime import date
from pathlib import Path

from .specification import _changes, _decode, _error, _json, _source_plan_repair_binding, update_specification
from ..contracts.plan import render_plan_contract, validate_plan_contract
from ..contracts.task import validate_task_contract
from ..contracts.validation import nonempty_string, strict_keys
from ..foundation.fingerprint import raw_sha256
from ..foundation.paths import validate_artifact_paths
from ..foundation.spec_update import storage_path, require_no_spec_update, require_idle_writer


PLAN_FIELDS = {"title", "summary", "goals", "scope", "deliverables", "acceptance_criteria"}
TASK_FIELDS = {"title", "summary", "decisions", "execution_defaults"}
ROW_FIELDS = {"title", "goal", "traceability", "dependencies", "steps", "validations", "commands", "operations"}


def prepare_specification(raw_request: bytes, *, project_root: Path,
                          user_config_root: str, skill_roots=None,
                          output_file: str | None = None,
                          migration: bool = False) -> dict[str, object]:
    request = strict_keys(_decode(raw_request), location="spec_prepare", required={
        "schema", "plan_path", "reason", "edits",
    } | ({"instruction_review", "instruction_choices"} if migration else set()),
        optional={"source_plan_repair"} if migration else set())
    if request["schema"] != ("work-migration-prepare-request/v1" if migration else "work-spec-prepare-request/v1"):
        raise _error("spec_prepare_schema", "Invalid specification preparation schema.")
    nonempty_string(request["reason"], location="reason")
    plan_path = nonempty_string(request["plan_path"], location="plan_path")
    original_plan = storage_path(project_root, plan_path).read_bytes()
    plan = _decode(original_plan)
    artifacts = validate_artifact_paths(project_root, plan.get("requirement_id"),
                                       plan.get("artifacts"), actual_plan_path=plan_path)
    require_no_spec_update(project_root, artifacts["execution"])
    require_idle_writer(project_root, artifacts["execution"])
    original = {"plan": original_plan,
                "task": storage_path(project_root, artifacts["task"]).read_bytes(),
                "index": storage_path(project_root, artifacts["execution"] + "/index.json").read_bytes()}
    options = dict(project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    repair_binding = None
    if migration and "source_plan_repair" in request:
        repair_binding = _source_plan_repair_binding(request["source_plan_repair"],
            task=_decode(original["task"]), plan_sha256=raw_sha256(original_plan))
    validate_plan_contract(original_plan, source="original Plan", actual_plan_path=plan_path,
                           _historical_work_sources=migration, **options)
    validate_task_contract(original["task"], source="original TASK", actual_task_path=artifacts["task"],
                           validate_file_state=False, _source_plan_raw=original_plan,
                           _historical_work_sources=migration,
                           _reviewed_source_plan_binding=repair_binding, **options)
    old_task = _decode(original["task"])
    task = copy.deepcopy(old_task)
    edits = request["edits"]
    if not isinstance(edits, list) or (not edits and not migration):
        raise _error("spec_prepare_edits", "Supply non-empty field replacements.")
    if migration:
        from .migration_prepare import instruction_edits
        if any(isinstance(edit, dict) and edit.get("field") in {
            "instruction_selection", "work_instruction_selection",
        } for edit in edits):
            raise _error("spec_prepare_field", "Supply instruction choices instead of snapshot edits.")
        edits = edits + instruction_edits(request["instruction_choices"], plan, task)
    seen = set()
    plan_changes = []
    today = date.today().isoformat()
    for edit in edits:
        edit = strict_keys(edit, location="edit", required={"artifact", "field", "before", "after"},
                           optional={"task_id", "affected_ids"})
        artifact = nonempty_string(edit["artifact"], location="edit.artifact")
        field = nonempty_string(edit["field"], location="edit.field")
        task_id = edit.get("task_id")
        if task_id is not None:
            nonempty_string(task_id, location="edit.task_id")
        identity = (artifact, task_id, field)
        if identity in seen:
            raise _error("spec_prepare_duplicate", "Replace each field only once.")
        seen.add(identity)
        if artifact == "plan" and task_id is None and field in (PLAN_FIELDS | ({"work_instruction_selection"} if migration else set())):
            target = plan
            if "affected_ids" not in edit:
                raise _error("spec_prepare_plan_evidence", "Plan edits require confirmed affected_ids.")
            plan_changes.append(edit)
        elif artifact == "task" and "affected_ids" not in edit:
            target = task
            allowed = TASK_FIELDS
            if task_id is not None:
                matches = [row for row in task["tasks"] if row["id"] == task_id]
                if len(matches) != 1:
                    raise _error("spec_prepare_task_id", "Unknown TASK ID.")
                target, allowed = matches[0], ROW_FIELDS
            if migration:
                allowed = allowed | {"instruction_selection"}
            if field not in allowed:
                raise _error("spec_prepare_field", "This field is not editable through preparation.")
        else:
            raise _error("spec_prepare_field", "This artifact or field is not editable through preparation.")
        if field not in target or _json(target[field]) != _json(edit["before"]):
            raise _error("spec_prepare_old_value", "The expected existing field value does not match.")
        if _json(edit["before"]) == _json(edit["after"]):
            raise _error("spec_prepare_unchanged", "Each replacement must change its field.")
        target[field] = copy.deepcopy(edit["after"])
    if plan_changes:
        history = plan.setdefault("changes", [])
        number = max((int(item["id"].rsplit("-", 1)[1]) for item in history), default=0)
        for offset, edit in enumerate(plan_changes, 1):
            history.append({"id": f"PLAN-CHANGE-{number + offset:03d}", "date": today,
                            "location": edit["field"], "before": _json(edit["before"]).decode().strip(),
                            "after": _json(edit["after"]).decode().strip(), "reason": request["reason"],
                            "affected_ids": edit["affected_ids"]})
    task["source_plan"]["canonical_sha256"] = raw_sha256(render_plan_contract(plan))
    task["spec_id"] = f"TASK-SPEC-{int(old_task['spec_id'].rsplit('-', 1)[1]) + 1:03d}"
    task["readiness"]["spec_id"] = task["spec_id"]
    number = max((int(item["id"].rsplit("-", 1)[1]) for item in old_task.get("changes", [])), default=0)
    task["changes"] = [{"id": f"TASK-CHANGE-{number + 1:03d}", "spec_id": task["spec_id"],
                        "date": today, "reason": request["reason"],
                        "affected_ids": [row["id"] for row in task["tasks"]], "edits": _changes(old_task, task)}]
    if plan_changes:
        task["changes"][0]["plan_change_ids"] = [item["id"] for item in plan["changes"][-len(plan_changes):]]
    prepared = {"schema": "work-spec-update-request/v1", "reason": request["reason"],
                "expected": {key + "_sha256": raw_sha256(raw) for key, raw in original.items()},
                "plan": plan, "task": task}
    if migration:
        prepared["schema"] = "work-spec-migration-request/v1"
        prepared["instruction_review"] = request["instruction_review"]
        if "source_plan_repair" in request:
            prepared["source_plan_repair"] = request["source_plan_repair"]
    preview = update_specification(_json(prepared), migration=migration, **options)
    # Reuse the publisher's dependency analysis instead of maintaining another one.
    if preview["affected_task_ids"]:
        task["changes"][0]["affected_ids"] = preview["affected_task_ids"]
        preview = update_specification(_json(prepared), migration=migration, **options)
    # Nested change evidence retains input key order when rendered. Return the
    # same ordering used for preview and disk transport so reserialization cannot
    # change the candidate bytes covered by approval.
    prepared = _decode(_json(prepared))
    if output_file is not None:
        # Exclusive creation cannot overwrite an input or a formal artifact.
        with Path(output_file).open("xb") as stream:
            stream.write(_json(prepared))
    return {"schema": "work-migration-prepare/v1" if migration else "work-spec-prepare/v1", "request": prepared, "preview": preview,
            "output_file": output_file}
