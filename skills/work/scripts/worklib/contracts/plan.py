from __future__ import annotations

import os
import re
from datetime import date
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, read_raw
from ..skills.catalog import SkillRoot
from ..skills.selection import validate_skill_selection
from ..hierarchy.selection import (
    order_hierarchy_selection,
    validate_hierarchy_selection,
)
from ..instructions.work_selection import validate_work_instruction_selection
from ..foundation.markdown import (
    parse_json_contract,
    parse_markdown_json_contract,
    render_markdown_json_contract,
    require_canonical_markdown_json_contract,
)
from ..foundation.paths import resolve_project_relative_path, validate_artifact_paths


ID_PREFIXES = {
    "goals": "GOAL",
    "scope": "SCOPE",
    "constraints": "CONSTRAINT",
    "dependencies": "DEPENDENCY",
    "risks": "RISK",
    "milestones": "MILESTONE",
    "deliverables": "DELIVERABLE",
    "acceptance_criteria": "ACCEPTANCE",
    "decisions": "PLAN-DECISION",
    "changes": "PLAN-CHANGE",
}
TOP_REQUIRED = {
    "schema",
    "requirement_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "hierarchy_selection",
    "work_instruction_selection",
    "skill_selection",
    "goals",
    "scope",
    "deliverables",
    "acceptance_criteria",
}
TOP_OPTIONAL = {
    "constraints",
    "dependencies",
    "risks",
    "milestones",
    "decisions",
    "changes",
}
TOP_FIELD_ORDER = (
    "schema",
    "requirement_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "hierarchy_selection",
    "work_instruction_selection",
    "skill_selection",
    "goals",
    "scope",
    "constraints",
    "dependencies",
    "risks",
    "milestones",
    "deliverables",
    "acceptance_criteria",
    "decisions",
    "changes",
)
ITEM_FIELD_ORDER = {
    "goals": ("id", "statement"),
    "scope": ("id", "kind", "statement", "goal_ids"),
    "constraints": ("id", "statement", "applies_to"),
    "dependencies": ("id", "statement", "applies_to"),
    "risks": ("id", "condition", "impact", "mitigation", "applies_to"),
    "milestones": ("id", "statement", "deliverable_ids"),
    "deliverables": ("id", "statement", "goal_ids", "acceptance_ids"),
    "acceptance_criteria": ("id", "statement", "deliverable_ids"),
    "decisions": ("id", "statement", "rationale", "applies_to"),
    "changes": ("id", "date", "location", "before", "after", "reason", "affected_ids"),
}


def _ordered_object(value: object, order: tuple[str, ...]) -> object:
    if not isinstance(value, dict):
        return value
    result = {key: value[key] for key in order if key in value}
    for key in sorted(set(value) - set(order)):
        result[key] = value[key]
    return result


def order_plan_contract(contract: dict[str, Any]) -> dict[str, Any]:
    ordered = _ordered_object(contract, TOP_FIELD_ORDER)
    assert isinstance(ordered, dict)
    if "artifacts" in ordered:
        ordered["artifacts"] = _ordered_object(
            ordered["artifacts"], ("plan", "task", "execution")
        )
    if "hierarchy_selection" in ordered:
        ordered["hierarchy_selection"] = order_hierarchy_selection(
            ordered["hierarchy_selection"]
        )
    if "work_instruction_selection" in ordered:
        instruction_selection = _ordered_object(
            ordered["work_instruction_selection"],
            (
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256",
            ),
        )
        if isinstance(instruction_selection, dict) and isinstance(
            instruction_selection.get("sources"), list
        ):
            instruction_selection["sources"] = [
                _ordered_object(
                    source,
                    ("kind", "logical_name", "canonical_sha256"),
                )
                for source in instruction_selection["sources"]
            ]
        ordered["work_instruction_selection"] = instruction_selection
    if "skill_selection" in ordered:
        skill_selection = _ordered_object(
            ordered["skill_selection"],
            ("schema", "decision", "skills", "selection_sha256"),
        )
        if isinstance(skill_selection, dict) and isinstance(skill_selection.get("skills"), list):
            skill_selection["skills"] = [
                _ordered_object(
                    skill,
                    (
                        "id", "name", "scope", "root", "source", "description",
                        "mode_support", "allow_implicit_invocation", "dependency_status",
                        "summary_sha256", "bundle_sha256", "recommendation_reason",
                    ),
                )
                for skill in skill_selection["skills"]
            ]
        ordered["skill_selection"] = skill_selection
    for key, field_order in ITEM_FIELD_ORDER.items():
        if isinstance(ordered.get(key), list):
            ordered[key] = [
                _ordered_object(item, field_order) for item in ordered[key]
            ]
    return ordered


def render_plan_contract(contract: dict[str, Any]) -> bytes:
    ordered = order_plan_contract(contract)
    title = ordered.get("title")
    return render_markdown_json_contract(
        title if isinstance(title, str) else str(title),
        ordered,
    )


def _strict_keys(
    value: object,
    *,
    location: str,
    required: set[str],
    optional: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "expected_object",
            "A JSON object is required.",
            {"location": location},
        )
    allowed = required | (optional or set())
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - allowed)
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": location, "missing": missing, "unknown": unknown},
        )
    return value


def _nonempty_string(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(
            ExitCode.CONTRACT,
            "empty_text_value",
            "A non-empty string is required.",
            {"location": location},
        )
    return value


def _string_array(value: object, *, location: str, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A non-empty string array is required.",
            {"location": location},
        )
    result = [_nonempty_string(item, location=f"{location}[]") for item in value]
    if len(result) != len(set(result)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_array_value",
            "Array values must be unique.",
            {"location": location},
        )
    return result


def _items(
    contract: dict[str, Any],
    key: str,
    *,
    required_fields: set[str],
    optional_fields: set[str] | None = None,
) -> list[dict[str, Any]]:
    value = contract.get(key)
    if not isinstance(value, list) or not value:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_item_array",
            "A present Plan item array must be non-empty.",
            {"location": key},
        )
    prefix = ID_PREFIXES[key]
    previous = 0
    result: list[dict[str, Any]] = []
    for index, raw_item in enumerate(value):
        item = _strict_keys(
            raw_item,
            location=f"{key}[{index}]",
            required={"id", *required_fields},
            optional=optional_fields,
        )
        item_id = _nonempty_string(item["id"], location=f"{key}[{index}].id")
        match = re.fullmatch(rf"{re.escape(prefix)}-(\d{{3}})", item_id)
        if not match or int(match.group(1)) <= previous:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_or_unsorted_id",
                "Plan item IDs must use the expected prefix and ascending numeric order.",
                {"location": key, "id": item_id},
            )
        previous = int(match.group(1))
        result.append(item)
    return result


def _validate_references(contract: dict[str, Any], items_by_id: dict[str, dict[str, Any]]) -> None:
    goals = _items(contract, "goals", required_fields={"statement"})
    scope = _items(
        contract,
        "scope",
        required_fields={"kind", "statement"},
        optional_fields={"goal_ids"},
    )
    deliverables = _items(
        contract,
        "deliverables",
        required_fields={"statement", "goal_ids", "acceptance_ids"},
    )
    acceptances = _items(
        contract,
        "acceptance_criteria",
        required_fields={"statement", "deliverable_ids"},
    )

    for item in goals:
        _nonempty_string(item["statement"], location=f"{item['id']}.statement")
    for item in scope:
        if item["kind"] not in {"in_scope", "out_of_scope"}:
            raise WorkError(ExitCode.CONTRACT, "invalid_scope_kind", "Invalid scope kind.")
        _nonempty_string(item["statement"], location=f"{item['id']}.statement")
        if item["kind"] == "in_scope" and "goal_ids" not in item:
            raise WorkError(
                ExitCode.CONTRACT,
                "in_scope_without_goals",
                "An in-scope item must reference at least one Goal.",
                {"id": item["id"]},
            )
        if "goal_ids" in item:
            _validate_typed_ids(item["goal_ids"], "GOAL", items_by_id, f"{item['id']}.goal_ids")

    deliverable_acceptances: dict[str, set[str]] = {}
    acceptance_deliverables: dict[str, set[str]] = {}
    referenced_goals: set[str] = set()
    for item in deliverables:
        _nonempty_string(item["statement"], location=f"{item['id']}.statement")
        goal_ids = _validate_typed_ids(
            item["goal_ids"], "GOAL", items_by_id, f"{item['id']}.goal_ids"
        )
        acceptance_ids = _validate_typed_ids(
            item["acceptance_ids"],
            "ACCEPTANCE",
            items_by_id,
            f"{item['id']}.acceptance_ids",
        )
        referenced_goals.update(goal_ids)
        deliverable_acceptances[item["id"]] = set(acceptance_ids)
    for item in acceptances:
        _nonempty_string(item["statement"], location=f"{item['id']}.statement")
        deliverable_ids = _validate_typed_ids(
            item["deliverable_ids"],
            "DELIVERABLE",
            items_by_id,
            f"{item['id']}.deliverable_ids",
        )
        acceptance_deliverables[item["id"]] = set(deliverable_ids)

    goal_ids = {item["id"] for item in goals}
    if referenced_goals != goal_ids:
        raise WorkError(
            ExitCode.CONTRACT,
            "untraced_goal",
            "Every Goal must be referenced by at least one Deliverable.",
            {"untraced_ids": sorted(goal_ids - referenced_goals)},
        )
    for deliverable_id, acceptance_ids in deliverable_acceptances.items():
        for acceptance_id in acceptance_ids:
            if deliverable_id not in acceptance_deliverables.get(acceptance_id, set()):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "deliverable_acceptance_mismatch",
                    "Deliverable and Acceptance references must be bidirectional.",
                    {"deliverable_id": deliverable_id, "acceptance_id": acceptance_id},
                )
    for acceptance_id, deliverable_ids in acceptance_deliverables.items():
        for deliverable_id in deliverable_ids:
            if acceptance_id not in deliverable_acceptances.get(deliverable_id, set()):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "deliverable_acceptance_mismatch",
                    "Deliverable and Acceptance references must be bidirectional.",
                    {"deliverable_id": deliverable_id, "acceptance_id": acceptance_id},
                )


def _validate_typed_ids(
    value: object,
    prefix: str,
    items_by_id: dict[str, dict[str, Any]],
    location: str,
) -> list[str]:
    ids = _string_array(value, location=location)
    numbers: list[int] = []
    for item_id in ids:
        match = re.fullmatch(rf"{re.escape(prefix)}-(\d{{3}})", item_id)
        if not match or item_id not in items_by_id:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_reference",
                "A referenced Plan ID does not exist or has the wrong type.",
                {"location": location, "id": item_id},
            )
        numbers.append(int(match.group(1)))
    if numbers != sorted(numbers):
        raise WorkError(
            ExitCode.CONTRACT,
            "unsorted_references",
            "Referenced IDs must use ascending numeric order.",
            {"location": location},
        )
    return ids


def validate_plan_contract(
    raw: bytes,
    *,
    source: str,
    actual_plan_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    markdown_title, contract = parse_markdown_json_contract(raw, source=source)
    _strict_keys(
        contract,
        location="plan",
        required=TOP_REQUIRED,
        optional=TOP_OPTIONAL,
    )
    if contract["schema"] != "work-plan/v1":
        raise WorkError(ExitCode.CONTRACT, "invalid_plan_schema", "Invalid Plan schema.")
    if contract["status"] != "confirmed":
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_plan_status",
            "A formal Plan status must be confirmed.",
        )
    title = _nonempty_string(contract["title"], location="title")
    if "\n" in title or "\r" in title:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_plan_title",
            "The Plan title must fit on one line.",
        )
    _nonempty_string(contract["summary"], location="summary")
    if title != markdown_title:
        raise WorkError(
            ExitCode.CONTRACT,
            "plan_title_mismatch",
            "The Markdown H1 and Plan title must match.",
        )

    validate_artifact_paths(
        project_root,
        contract["requirement_id"],
        contract["artifacts"],
        actual_plan_path=actual_plan_path,
    )
    hierarchy_validation = validate_hierarchy_selection(
        contract["hierarchy_selection"],
        skill_root=Path(__file__).resolve().parents[3],
    )
    hierarchy_selection = hierarchy_validation["hierarchy_selection"]
    assert isinstance(hierarchy_selection, dict)
    confirmed_paths = hierarchy_selection["selected_paths"]
    assert isinstance(confirmed_paths, list)
    instruction_sources = validate_work_instruction_selection(
        contract["work_instruction_selection"],
        skill_root=Path(__file__).resolve().parents[3],
        mode="plan",
        selected_paths=confirmed_paths,
    )
    skill_validation = validate_skill_selection(
        contract["skill_selection"], roots=skill_roots or []
    )

    item_groups: dict[str, list[dict[str, Any]]] = {}
    field_sets = {
        "goals": ({"statement"}, set()),
        "scope": ({"kind", "statement"}, {"goal_ids"}),
        "deliverables": ({"statement", "goal_ids", "acceptance_ids"}, set()),
        "acceptance_criteria": ({"statement", "deliverable_ids"}, set()),
        "constraints": ({"statement", "applies_to"}, set()),
        "dependencies": ({"statement", "applies_to"}, set()),
        "risks": ({"condition", "impact", "mitigation", "applies_to"}, set()),
        "milestones": ({"statement", "deliverable_ids"}, set()),
        "decisions": ({"statement", "rationale", "applies_to"}, set()),
        "changes": (
            {"date", "location", "before", "after", "reason", "affected_ids"},
            set(),
        ),
    }
    for key, (required, optional) in field_sets.items():
        if key in contract:
            item_groups[key] = _items(
                contract,
                key,
                required_fields=required,
                optional_fields=optional,
            )

    items_by_id: dict[str, dict[str, Any]] = {}
    for items in item_groups.values():
        for item in items:
            if item["id"] in items_by_id:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "duplicate_plan_id",
                    "Plan IDs must be globally unique.",
                    {"id": item["id"]},
                )
            items_by_id[item["id"]] = item

    _validate_references(contract, items_by_id)
    for key in ("constraints", "dependencies"):
        for item in item_groups.get(key, []):
            _nonempty_string(item["statement"], location=f"{item['id']}.statement")
            _validate_applies_to(item["applies_to"], items_by_id, item["id"])
    for item in item_groups.get("risks", []):
        for field in ("condition", "impact", "mitigation"):
            _nonempty_string(item[field], location=f"{item['id']}.{field}")
        _validate_applies_to(item["applies_to"], items_by_id, item["id"])
    for item in item_groups.get("milestones", []):
        _nonempty_string(item["statement"], location=f"{item['id']}.statement")
        _validate_typed_ids(
            item["deliverable_ids"],
            "DELIVERABLE",
            items_by_id,
            f"{item['id']}.deliverable_ids",
        )
    for item in item_groups.get("decisions", []):
        _nonempty_string(item["statement"], location=f"{item['id']}.statement")
        _nonempty_string(item["rationale"], location=f"{item['id']}.rationale")
        _validate_applies_to(item["applies_to"], items_by_id, item["id"])
    for item in item_groups.get("changes", []):
        for field in ("location", "before", "after", "reason"):
            _nonempty_string(item[field], location=f"{item['id']}.{field}")
        try:
            date.fromisoformat(_nonempty_string(item["date"], location=f"{item['id']}.date"))
        except ValueError as error:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_change_date",
                "A Plan change date must use YYYY-MM-DD.",
                {"id": item["id"]},
            ) from error
        affected = _string_array(item["affected_ids"], location=f"{item['id']}.affected_ids")
        if any(reference not in items_by_id for reference in affected):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_reference",
                "A Plan change references an unknown affected ID.",
                {"id": item["id"]},
            )

    ordered_contract = order_plan_contract(contract)
    require_canonical_markdown_json_contract(
        raw,
        title=title,
        contract=ordered_contract,
        source=source,
    )
    return {
        "schema": "work-plan-validation/v1",
        "requirement_id": contract["requirement_id"],
        "status": contract["status"],
        "plan_sha256": canonical_sha256(raw, source=source),
        "hierarchy_selection_sha256": hierarchy_selection["selection_sha256"],
        "work_instructions_sha256": instruction_sources.instructions_sha256,
        "skill_selection_sha256": skill_validation["skill_selection"]["selection_sha256"],
        "item_count": len(items_by_id),
    }


def prepare_plan_json_contract(
    raw: bytes,
    *,
    source: str,
    actual_plan_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[dict[str, object], bytes]:
    contract = parse_json_contract(raw, source=source)
    rendered = render_plan_contract(contract)
    validation = validate_plan_contract(
        rendered,
        source=source,
        actual_plan_path=actual_plan_path,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    return validation, rendered


def validate_plan_json_contract(
    raw: bytes,
    *,
    source: str,
    actual_plan_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    validation, _ = prepare_plan_json_contract(
        raw,
        source=source,
        actual_plan_path=actual_plan_path,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    return validation


def create_plan_file(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    normalized, path = resolve_project_relative_path(
        project_root,
        raw_plan_path,
        field="plan_path",
    )
    validation, rendered = prepare_plan_json_contract(
        raw,
        source=source,
        actual_plan_path=normalized,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        )
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as output:
            output.write(rendered)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "plan_create_failed",
            "The canonical Plan could not be created.",
            {"path": normalized},
        ) from error

    stored = validate_plan_file(
        project_root, user_config_root, normalized, skill_roots=skill_roots
    )
    if stored != validation:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "plan_post_write_mismatch",
            "The stored Plan does not match the validated canonical Plan.",
            {"path": normalized},
        )
    result = dict(stored)
    result["schema"] = "work-plan-create/v1"
    result["path"] = normalized
    return result


def _validate_applies_to(
    value: object,
    items_by_id: dict[str, dict[str, Any]],
    owner_id: str,
) -> None:
    references = _string_array(value, location=f"{owner_id}.applies_to")
    for reference in references:
        if reference != "PLAN" and reference not in items_by_id:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_reference",
                "An applies_to reference is unknown.",
                {"id": owner_id, "reference": reference},
            )


def validate_plan_file(
    project_root: Path,
    user_config_root: str,
    raw_path: str,
    *,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    normalized, path = resolve_project_relative_path(
        project_root, raw_path, field="plan_path"
    )
    raw = read_raw(path)
    return validate_plan_contract(
        raw,
        source=str(path),
        actual_plan_path=normalized,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
