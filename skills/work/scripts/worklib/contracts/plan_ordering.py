from __future__ import annotations

from typing import Any

from ..hierarchy.ordering import order_hierarchy_selection


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
        if isinstance(skill_selection, dict) and isinstance(
            skill_selection.get("skills"), list
        ):
            skill_selection["skills"] = [
                _ordered_object(
                    skill,
                    (
                        "id",
                        "name",
                        "scope",
                        "root",
                        "source",
                        "description",
                        "mode_support",
                        "allow_implicit_invocation",
                        "dependency_status",
                        "summary_sha256",
                        "bundle_sha256",
                        "recommendation_reason",
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
