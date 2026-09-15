"""Prepare a migration from reviewed edits and explicit instruction choices."""
from __future__ import annotations

from pathlib import Path

from .specification import _decode, _error, _json
from .migration_preflight import migration_preflight
from ..contracts.plan import ID_PREFIXES
from ..contracts.validation import strict_keys
from ..foundation.runtime import installed_work_root
from ..foundation.spec_update import storage_path
from ..instructions.selection import build_instruction_selection
from ..instructions.task_selection import build_task_document_instruction_selection


def instruction_edits(value, plan, task):
    choices = strict_keys(value, location="instruction_choices", required={"plan", "tasks"})
    rows = strict_keys(choices["tasks"], location="instruction_choices.tasks",
                       required={row["id"] for row in task["tasks"]})
    root = installed_work_root()
    edits = []

    def build(choice, mode):
        choice = strict_keys(choice, location="instruction_choice", required={"selected_paths", "references"})
        return build_instruction_selection(skill_root=root, mode=mode,
            selected_paths=choice["selected_paths"], reference_names=choice["references"])

    def replace(artifact, field, before, after, **extra):
        if before != after:
            edits.append(dict(artifact=artifact, field=field, before=before, after=after, **extra))

    replace("plan", "work_instruction_selection", plan["work_instruction_selection"],
            build(choices["plan"], "plan"), affected_ids=[
                item["id"] for group in ID_PREFIXES if group != "changes"
                for item in plan.get(group, [])
            ])
    selections = []
    for row in task["tasks"]:
        selection = build(rows[row["id"]], "task")
        selections.append(selection)
        replace("task", "instruction_selection", row["instruction_selection"], selection, task_id=row["id"])
    replace("task", "instruction_selection", task["instruction_selection"],
            build_task_document_instruction_selection(selections, skill_root=root))
    return edits


def prepare_migration(raw_request, *, project_root, user_config_root, skill_roots=None, output_file=None):
    from .spec_prepare import prepare_specification

    request = strict_keys(_decode(raw_request), location="migration_prepare", required={
        "schema", "plan_path", "reason", "edits", "instruction_choices", "instruction_review",
    }, optional={"source_plan_repair"})
    if request["schema"] != "work-migration-prepare-request/v1":
        raise _error("spec_prepare_schema", "Invalid migration preparation schema.",
                     expected_schema="work-migration-prepare-request/v1",
                     hint="Use the preparation request schema for the selected command.")
    plan = _decode(storage_path(project_root, request["plan_path"]).read_bytes())
    options = dict(project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots)
    preflight = migration_preflight(_json({"schema": "work-migration-preflight-request/v1",
        "requirement_id": plan.get("requirement_id"), "artifacts": plan.get("artifacts")}), **options)
    # Preflight grants no repair exception. The publisher validates explicit
    # baseline evidence and every remaining contract/transaction check below.
    if not preflight["can_prepare_candidate"] and "source_plan_repair" not in request:
        raise _error("migration_preflight_blocked", "Review migration prerequisites.", preflight=preflight)
    result = prepare_specification(raw_request, migration=True, **options)
    if result["request"]["expected"] != {
        key + "_sha256": value["raw_sha256"] for key, value in preflight["fingerprints"].items()
    }:
        raise _error("spec_update_source_changed", "Sources changed after migration preflight.")
    if output_file is not None:
        with Path(output_file).open("xb") as stream:
            stream.write(_json(result["request"]))
    result["output_file"] = output_file
    result["preflight"] = preflight
    return result
