"""Turn specification choices into formal edits using validated source records."""

from __future__ import annotations

import copy
import re
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.plan import ID_PREFIXES


PLAN_GROUPS = {
    "goals": ({"statement"}, set()),
    "scope": ({"kind", "statement"}, {"goal_keys", "goal_positions"}),
    "constraints": ({"statement", "applies_to"}, set()),
    "dependencies": ({"statement", "applies_to"}, set()),
    "risks": ({"condition", "impact", "mitigation", "applies_to"}, set()),
    "milestones": ({"statement"}, {"deliverable_keys", "deliverable_positions"}),
    "deliverables": ({"statement"}, {"goal_keys", "goal_positions", "acceptance_keys", "acceptance_positions"}),
    "acceptance_criteria": ({"statement"}, {"deliverable_keys", "deliverable_positions"}),
    "decisions": ({"statement", "rationale", "applies_to"}, set()),
}
TASK_GROUPS = {"inputs", "decisions", "files", "risks", "steps", "commands", "operations", "validations"}
_LOCAL_KEY = re.compile(r"[a-z][a-z0-9_-]*")


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, details)


def _position(value: object, size: int, *, location: str) -> int:
    if type(value) is not int or value < 1 or value > size:
        _fail("invalid_semantic_position", "A one-based position must identify a source or candidate item.", location=location)
    return value


def _key(value: object, *, location: str) -> str:
    if not isinstance(value, str) or _LOCAL_KEY.fullmatch(value) is None:
        _fail("invalid_semantic_key", "Use a local lowercase semantic key.", location=location)
    return value


def _object(value: object, *, required: set[str], optional: set[str] | None = None, location: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        _fail("invalid_semantic_object", "A semantic object is required.", location=location)
    missing = required - set(value)
    unknown = set(value) - required - (optional or set())
    if missing or unknown:
        _fail("invalid_object_fields", "The semantic object has missing or unknown fields.", location=location,
              missing=sorted(missing), unknown=sorted(unknown))
    return value


def _allocate_rows(group: str, source: list[dict[str, Any]], value: object, *, prefix: str,
                   required: set[str], optional: set[str]) -> tuple[list[dict[str, Any]], dict[str, str]]:
    if not isinstance(value, list):
        _fail("invalid_semantic_items", "A semantic collection must be an array.", collection=group)
    next_number = max((int(row["id"].rsplit("-", 1)[1]) for row in source), default=0)
    used: set[int] = set()
    keys: dict[str, str] = {}
    rows: list[dict[str, Any]] = []
    for index, raw in enumerate(value, 1):
        row = _object(raw, required=required | {"key"}, optional=optional | {"existing_position"},
                      location=f"{group}[{index}]")
        key = _key(row["key"], location=f"{group}[{index}].key")
        if key in keys:
            _fail("duplicate_semantic_key", "Local semantic keys must be unique.", collection=group, key=key)
        existing = row.get("existing_position")
        if existing is not None:
            existing = _position(existing, len(source), location=f"{group}[{index}].existing_position")
            if existing in used:
                _fail("duplicate_semantic_position", "An existing item may be retained once.", collection=group)
            used.add(existing)
            record_id = source[existing - 1]["id"]
        else:
            next_number += 1
            record_id = f"{prefix}-{next_number:03d}"
        keys[key] = record_id
        rows.append({"id": record_id, **{name: copy.deepcopy(item) for name, item in row.items()
                                       if name not in {"key", "existing_position"}}})
    return rows, keys


def build_plan_collections(plan: dict[str, Any], changes: dict[str, object]) -> dict[str, object]:
    """Allocate Plan IDs and resolve all local cross references together."""
    results: dict[str, object] = {}
    keys: dict[str, dict[str, str]] = {}
    for group, (required, optional) in PLAN_GROUPS.items():
        if group not in changes:
            results[group] = copy.deepcopy(plan.get(group))
            keys[group] = {}
        elif changes[group] is None:
            if group in {"goals", "scope", "deliverables", "acceptance_criteria"}:
                _fail("spec_semantic_required_collection", "A required Plan collection cannot be removed.", collection=group)
            results[group], keys[group] = None, {}
        else:
            results[group], keys[group] = _allocate_rows(
                group, plan.get(group) or [], changes[group], prefix=ID_PREFIXES[group],
                required=required, optional=optional,
            )

    def refs(row: dict[str, Any], prefix: str, group: str, *, required: bool) -> list[str] | None:
        key_name, position_name = f"{prefix}_keys", f"{prefix}_positions"
        if key_name not in row and position_name not in row:
            if required:
                _fail("invalid_semantic_reference", "A required Plan relation is missing.", collection=group)
            return None
        values: list[str] = []
        if key_name in row:
            if not isinstance(row[key_name], list):
                _fail("invalid_semantic_reference", "Semantic keys must be an array.", collection=group)
            for key in row.pop(key_name):
                name = _key(key, location=key_name)
                if name not in keys[group]:
                    _fail("invalid_semantic_reference", "A local Plan key is unknown.", collection=group, key=name)
                values.append(keys[group][name])
        if position_name in row:
            positions = row.pop(position_name)
            if not isinstance(positions, list):
                _fail("invalid_semantic_position", "Semantic positions must be an array.", collection=group)
            target = results[group] or []
            for position in positions:
                values.append(target[_position(position, len(target), location=position_name) - 1]["id"])
        if len(values) != len(set(values)) or (required and not values):
            _fail("invalid_semantic_reference", "Semantic Plan relations must be nonempty and unique.", collection=group)
        return values

    def applies(row: dict[str, Any]) -> list[str]:
        values = row.pop("applies_to")
        if not isinstance(values, list) or not values:
            _fail("invalid_semantic_reference", "applies_to requires semantic references.")
        result = []
        for raw in values:
            ref = _object(raw, required={"collection"}, optional={"key", "position"}, location="applies_to")
            group = ref["collection"]
            if group == "plan" and set(ref) == {"collection"}:
                result.append("PLAN")
            elif group in PLAN_GROUPS and ("key" in ref) != ("position" in ref):
                if "key" in ref:
                    key = _key(ref["key"], location="applies_to.key")
                    if key not in keys[group]:
                        _fail("invalid_semantic_reference", "A local Plan key is unknown.", collection=group, key=key)
                    result.append(keys[group][key])
                else:
                    target = results[group] or []
                    result.append(target[_position(ref["position"], len(target), location="applies_to.position") - 1]["id"])
            else:
                _fail("invalid_semantic_reference", "applies_to must identify one semantic Plan item.")
        if len(result) != len(set(result)):
            _fail("invalid_semantic_reference", "applies_to references must be unique.")
        return result

    for group in changes:
        rows = results[group]
        if rows is None:
            continue
        for row in rows:
            if group == "scope":
                goal_ids = refs(row, "goal", "goals", required=row["kind"] == "in_scope")
                if goal_ids is not None:
                    row["goal_ids"] = goal_ids
            elif group == "deliverables":
                row["goal_ids"] = refs(row, "goal", "goals", required=True)
                row["acceptance_ids"] = refs(row, "acceptance", "acceptance_criteria", required=True)
            elif group == "acceptance_criteria":
                row["deliverable_ids"] = refs(row, "deliverable", "deliverables", required=True)
            elif group == "milestones":
                row["deliverable_ids"] = refs(row, "deliverable", "deliverables", required=True)
            elif group in {"constraints", "dependencies", "risks", "decisions"}:
                row["applies_to"] = applies(row)
    return {group: results[group] for group in changes}


def _task_positions(positions: object, task_ids: list[str], *, location: str) -> list[str]:
    if not isinstance(positions, list):
        _fail("invalid_semantic_position", "TASK positions must be an array.", location=location)
    result = [task_ids[_position(value, len(task_ids), location=location) - 1] for value in positions]
    if len(result) != len(set(result)):
        _fail("invalid_semantic_position", "TASK positions must be unique.", location=location)
    return result


def _traceability(value: object, plan: dict[str, Any]) -> dict[str, list[str]]:
    row = _object(value, required={"goal_positions", "deliverable_positions", "acceptance_positions"},
                  optional={"milestone_positions"}, location="traceability")
    result = {}
    for name, group in (("goal", "goals"), ("deliverable", "deliverables"),
                        ("acceptance", "acceptance_criteria"), ("milestone", "milestones")):
        key = f"{name}_positions"
        if key not in row:
            continue
        positions = row[key]
        if not isinstance(positions, list) or not positions:
            _fail("invalid_semantic_position", "Traceability positions must be nonempty arrays.", location=key)
        items = plan.get(group) or []
        ids = [items[_position(value, len(items), location=key) - 1]["id"] for value in positions]
        if len(ids) != len(set(ids)):
            _fail("invalid_semantic_position", "Traceability positions must be unique.", location=key)
        result[f"{name}_ids"] = ids
    return result


def _index_decisions(value: object, index: dict[str, Any]) -> list[dict[str, Any]] | None:
    if value is None:
        return None
    rows, _ = _allocate_rows("decisions", index.get("decisions") or [], value,
                              prefix="TASK-DECISION", required={"statement", "rationale", "task_positions"}, optional=set())
    task_ids = [reference["id"] for reference in index["tasks"]]
    for row in rows:
        row["task_ids"] = _task_positions(row.pop("task_positions"), task_ids, location="task_positions")
    return rows


def formalize_specification_edits(semantic_edits: list[dict[str, Any]], baseline: dict[str, Any],
                                  task_operations, *, skill_root, task_candidate_builder,
                                  task_patch_builder) -> list[dict[str, Any]]:
    """Derive every formal edit from a validated collection and semantic choices."""
    plan_changes = {edit["field"]: edit["semantic_after"] for edit in semantic_edits
                    if (edit.get("target") or {}).get("artifact") == "plan" and "semantic_after" in edit}
    plan_values = build_plan_collections(baseline["plan"], plan_changes)
    plan = copy.deepcopy(baseline["plan"])
    for edit in semantic_edits:
        if (edit.get("target") or {}).get("artifact") == "plan":
            field = edit["field"]
            after = plan_values[field] if field in plan_values else edit["after"]
            if after is None:
                plan.pop(field, None)
            else:
                plan[field] = copy.deepcopy(after)
    task_ids = [reference["id"] for reference in baseline["index"]["tasks"]]
    nested: dict[str, dict[str, Any]] = {}
    for edit in semantic_edits:
        target = edit.get("target") or {}
        if target.get("artifact") == "task_item" and edit.get("field") in TASK_GROUPS:
            nested.setdefault(target["task_id"], {})[edit["field"]] = edit["semantic_after"]
    nested_values = {}
    for task_id, replacements in nested.items():
        current = baseline["items"].get(task_id)
        if current is None:
            _fail("spec_prepare_task_id", "Unknown TASK ID.", task_id=task_id)
        dependency_ids = current.get("dependencies") or []
        dependency_files = {dep_id: {f"existing-{position}": row["id"] for position, row in enumerate(baseline["items"][dep_id].get("files") or [], 1)}
                            for dep_id in dependency_ids}
        nested_values[task_id] = task_patch_builder(current, replacements,
            acceptance_ids=[row["id"] for row in plan["acceptance_criteria"]],
            dependency_ids=dependency_ids, dependency_files=dependency_files)
    formal: list[dict[str, Any]] = []
    available_items = dict(baseline["items"])
    next_task = max((int(task_id.rsplit("-", 1)[1]) for task_id in task_ids), default=0)
    for edit in semantic_edits:
        operation = edit.get("operation")
        if operation == "add_task":
            next_task += 1
            task_id = f"TASK-{next_task:03d}"
            candidate = edit["task"]
            dependencies = _task_positions(candidate.get("dependency_positions", []), task_ids, location="dependency_positions")
            if task_id in dependencies:
                _fail("invalid_semantic_reference", "A TASK cannot depend on itself.")
            selections = task_operations.build_instruction_selection(
                skill_root=skill_root, mode="task", selected_paths=candidate["selected_paths"],
                reference_names=candidate["references"],
            )
            dependency_files = {dep_id: {f"existing-{position}": row["id"] for position, row in enumerate(available_items[dep_id].get("files") or [], 1)}
                                for dep_id in dependencies}
            nested_candidate = task_candidate_builder(candidate["candidate"],
                acceptance_ids=[row["id"] for row in plan["acceptance_criteria"]],
                dependency_ids=dependencies, dependency_files=dependency_files)
            item = {"schema": "work-task-item/v1", "id": task_id, "title": candidate["title"],
                    "goal": candidate["goal"], "skill_id": candidate["skill_id"],
                    "instruction_selection": selections,
                    "traceability": {"goal_ids": [row["id"] for row in plan["goals"]],
                                     "deliverable_ids": [row["id"] for row in plan["deliverables"]],
                                     "acceptance_ids": [row["id"] for row in plan["acceptance_criteria"]]},
                    **({"dependencies": dependencies} if dependencies else {}), **nested_candidate}
            formal.append({"artifact": "task_item", "task_id": task_id, "operation": "add", "path": "/", "after": item})
            available_items[task_id] = item
            task_ids.append(task_id)
            continue
        if operation == "remove_task":
            position = _position(edit["task_position"], len(baseline["index"]["tasks"]), location="task_position")
            task_id = baseline["index"]["tasks"][position - 1]["id"]
            if any(row.get("id") == task_id and (row.get("latest_attempt") or row["status"] != "pending") for row in baseline["execution"]["tasks"]):
                _fail("spec_remove_task_history", "A TASK with execution history requires a dedicated lifecycle operation.")
            formal.append({"artifact": "task_item", "task_id": task_id, "operation": "remove", "path": "/",
                           "before": copy.deepcopy(baseline["items"][task_id])})
            continue
        target, field = edit["target"], edit["field"]
        artifact, task_id = target["artifact"], target.get("task_id")
        source = baseline["plan"] if artifact == "plan" else baseline["index"] if artifact == "task_index" else baseline["items"].get(task_id)
        if source is None:
            _fail("spec_prepare_task_id", "Unknown TASK ID.", task_id=task_id)
        if "after" in edit:
            after = edit["after"]
        elif artifact == "plan":
            after = plan_values[field]
        elif artifact == "task_index" and field == "decisions":
            after = _index_decisions(edit["semantic_after"], baseline["index"])
        elif artifact == "task_index" and field == "execution_defaults":
            after = None if edit["semantic_after"] is None else _object(edit["semantic_after"],
                required={"working_directory", "os", "shell"}, location="execution_defaults")
        elif field in TASK_GROUPS:
            after = nested_values[task_id][field]
        elif field == "traceability":
            after = _traceability(edit["semantic_after"], plan)
        elif field == "dependencies":
            after = _task_positions(edit["semantic_after"], task_ids, location="dependency_positions")
        else:
            _fail("spec_prepare_field", "This semantic field is unsupported.", field=field)
        present = field in source
        before = copy.deepcopy(source.get(field)) if present else None
        if (not present and after is None) or (present and before == after):
            _fail("spec_edit_state", "A specification edit must change its target.", field=field)
        row = {"artifact": artifact, "operation": "remove" if after is None else "replace" if present else "add",
               "path": "/" + field}
        if task_id is not None:
            row["task_id"] = task_id
        if artifact == "plan":
            row["affected_ids"] = sorted({item["id"] for group in PLAN_GROUPS for item in (plan.get(group) or [])})
        if present:
            row["before"] = before
        if after is not None:
            row["after"] = copy.deepcopy(after)
        formal.append(row)
    return formal
