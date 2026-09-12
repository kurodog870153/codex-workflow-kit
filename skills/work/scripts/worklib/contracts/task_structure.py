"""Independent TASK shape checks; no source loading or candidate repair."""

from __future__ import annotations

from typing import Any

TOP_REQUIRED = {
    "schema", "requirement_id", "spec_id", "status", "title", "summary",
    "artifacts", "source_plan", "instruction_selection", "tasks", "readiness",
}
TOP_OPTIONAL = {"execution_defaults", "decisions", "changes"}
TASK_REQUIRED = {
    "id", "title", "skill_id", "instruction_selection", "traceability",
    "goal", "steps", "validations",
}
TASK_OPTIONAL = {"dependencies", "inputs", "decisions", "files", "risks", "commands", "operations"}

DECISION_FIELDS = {
    "title", "summary", "goal", "steps", "validations", "traceability",
    "statement", "rationale", "criteria", "pass_condition", "acceptance_ids",
    "goal_ids", "deliverable_ids", "action", "references",
}
LEGACY_FIELDS = {"rule_selection", "rules_sha256", "instructions_sha256"}


def pointer(parent: str, key: object) -> str:
    return parent + "/" + str(key).replace("~", "~0").replace("/", "~1")


def inspect_task_structure(contract: dict[str, Any]) -> list[dict[str, Any]]:
    issues: list[dict[str, Any]] = []

    def issue(code, location, field="", **details):
        category = "decision_required" if field in DECISION_FIELDS else "review_required"
        if code == "legacy_field":
            category = "migration_review"
        issues.append({
            "stage": "structure", "code": code, "location": location,
            "category": category,
            "message": {
                "missing_field": "A required field is missing.",
                "unknown_field": "An unrecognized field must be reviewed, not discarded.",
                "legacy_field": "A known legacy field needs an explicitly reviewed conversion.",
                "invalid_type": "The field has the wrong JSON type.",
                "empty_array": "The array must not be empty.",
                "empty_text": "Required text needs a confirmed value.",
            }[code],
            "suggestion": (
                "Discuss the missing content after format repair; do not insert placeholders."
                if category == "decision_required"
                else "Review the original value and the expected contract before proposing a change."
            ),
            "details": details,
        })

    def obj(value, required, optional, location):
        if not isinstance(value, dict):
            issue("invalid_type", location, expected="object")
            return None
        for key in sorted(required - value.keys()):
            issue("missing_field", pointer(location, key), key)
        for key in sorted(value.keys() - required - optional):
            legacy = key in LEGACY_FIELDS and (location == "" or location.startswith("/tasks/"))
            issue("legacy_field" if legacy else "unknown_field", pointer(location, key), key)
        return value

    def texts(value, fields, location, nullable=()):
        for key in fields & value.keys():
            item = value[key]
            if item is None and key in nullable:
                continue
            if not isinstance(item, str):
                issue("invalid_type", pointer(location, key), key, expected="string")
            elif not item.strip():
                issue("empty_text", pointer(location, key), key)

    def strings(value, location, field="", allow_empty=True):
        if not isinstance(value, list):
            issue("invalid_type", location, field, expected="array")
            return
        if not value and not allow_empty:
            issue("empty_array", location, field)
        for index, item in enumerate(value):
            if not isinstance(item, str):
                issue("invalid_type", pointer(location, index), field, expected="string")
            elif not item.strip():
                issue("empty_text", pointer(location, index), field)

    def array(value, location, field=""):
        if not isinstance(value, list):
            issue("invalid_type", location, field, expected="array")
            return []
        if not value:
            issue("empty_array", location, field)
        return list(enumerate(value))

    def selection(value, location, document=False):
        fields = {"sources", "references", "instructions_sha256"}
        if not document:
            fields |= {"selected_paths", "resolved_paths"}
        value = obj(value, fields, set(), location)
        if value is None:
            return
        texts(value, {"instructions_sha256"}, location)
        for key in {"references", "selected_paths", "resolved_paths"} & value.keys():
            strings(value[key], pointer(location, key), key, allow_empty=key != "resolved_paths")
        if "sources" in value:
            for index, source in array(value["sources"], pointer(location, "sources")):
                path = pointer(pointer(location, "sources"), index)
                source = obj(source, {"kind", "logical_name", "canonical_sha256"}, set(), path)
                if source is not None:
                    texts(source, {"kind", "logical_name", "canonical_sha256"}, path)

    def execution(value, location):
        value = obj(value, {"working_directory", "os", "shell"}, set(), location)
        if value is not None:
            texts(value, {"working_directory", "os", "shell"}, location)

    obj(contract, TOP_REQUIRED, TOP_OPTIONAL, "")
    texts(contract, {"schema", "requirement_id", "spec_id", "status", "title", "summary"}, "")
    for key, fields in (
        ("artifacts", {"plan", "task", "execution"}),
        ("source_plan", {"canonical_sha256", "hierarchy_selection_sha256"}),
        ("readiness", {"status", "spec_id"}),
    ):
        if key in contract:
            value = obj(contract[key], fields, set(), "/" + key)
            if value is not None:
                texts(value, fields, "/" + key)
    if "instruction_selection" in contract:
        selection(contract["instruction_selection"], "/instruction_selection", document=True)
    if "execution_defaults" in contract:
        execution(contract["execution_defaults"], "/execution_defaults")

    for key, required, optional in (
        ("decisions", {"id", "statement", "rationale", "task_ids"}, set()),
        ("changes", {"id", "spec_id", "date", "reason", "affected_ids", "edits"}, {"plan_change_ids"}),
    ):
        if key not in contract:
            continue
        for index, value in array(contract[key], "/" + key, key):
            location = pointer("/" + key, index)
            value = obj(value, required, optional, location)
            if value is None:
                continue
            texts(value, required - {"task_ids", "affected_ids", "edits"}, location)
            for field in {"task_ids", "affected_ids", "plan_change_ids"} & value.keys():
                strings(value[field], pointer(location, field), field, allow_empty=False)
            if key == "changes" and "edits" in value:
                for edit_index, edit in array(value["edits"], pointer(location, "edits")):
                    edit_path = pointer(pointer(location, "edits"), edit_index)
                    edit = obj(edit, {"operation", "path"}, {"before", "after"}, edit_path)
                    if edit is not None:
                        texts(edit, {"operation", "path"}, edit_path)

    if "tasks" not in contract:
        return issues
    groups = {
        "inputs": ({"kind", "source", "precondition"}, set()),
        "decisions": ({"statement", "rationale"}, set()),
        "files": ({"action"}, {"path", "source", "destination"}),
        "risks": ({"condition", "impact", "mitigation"}, set()),
        "steps": ({"action", "references"}, set()),
        "commands": ({"mode"}, {"argv", "script", "execution"}),
        "operations": ({"kind", "action", "target", "validation_id"}, {"command_id"}),
        "validations": ({"kind"}, {"command_ids", "pass_condition", "confirmer", "criteria", "acceptance_ids"}),
    }
    for index, task in array(contract["tasks"], "/tasks", "tasks"):
        location = pointer("/tasks", index)
        task = obj(task, TASK_REQUIRED, TASK_OPTIONAL, location)
        if task is None:
            continue
        texts(task, {"id", "title", "goal", "skill_id"}, location, nullable={"skill_id"})
        if "instruction_selection" in task:
            selection(task["instruction_selection"], pointer(location, "instruction_selection"))
        if "dependencies" in task:
            strings(task["dependencies"], pointer(location, "dependencies"), "dependencies")
        if "traceability" in task:
            trace = obj(task["traceability"], {"goal_ids", "deliverable_ids", "acceptance_ids"},
                        {"milestone_ids"}, pointer(location, "traceability"))
            if trace is not None:
                for key in {"goal_ids", "deliverable_ids", "acceptance_ids", "milestone_ids"} & trace.keys():
                    strings(trace[key], pointer(pointer(location, "traceability"), key), key, allow_empty=False)
        for key, (required, optional) in groups.items():
            if key not in task:
                continue
            for item_index, item in array(task[key], pointer(location, key), key):
                path = pointer(pointer(location, key), item_index)
                item = obj(item, {"id"} | required, optional, path)
                if item is None:
                    continue
                texts(item, ({"id"} | required | optional) - {"argv", "references", "execution", "command_ids", "acceptance_ids"}, path)
                for field in {"argv", "references", "command_ids", "acceptance_ids"} & item.keys():
                    strings(item[field], pointer(path, field), field, allow_empty=False)
                if "execution" in item:
                    execution(item["execution"], pointer(path, "execution"))
                conditional = set()
                if key == "commands":
                    conditional = {"argv"} if item.get("mode") == "argv" else ({"script"} if item.get("mode") == "shell" else set())
                elif key == "validations":
                    conditional = {"command_ids", "pass_condition"} if item.get("kind") == "automated" else ({"confirmer", "criteria"} if item.get("kind") == "manual" else set())
                elif key == "files":
                    conditional = {"path"} if item.get("action") in ("create", "modify") else ({"source", "destination"} if item.get("action") == "move" else set())
                for field in sorted(conditional - item.keys()):
                    issue("missing_field", pointer(path, field), field)
    return issues
