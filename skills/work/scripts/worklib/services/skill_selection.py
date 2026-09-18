from __future__ import annotations

import json
import re
from typing import Any

from ..models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_json_sha256, decode_utf8
from ..protocol import SHA256_PATTERN as SHA256_PATTERN_TEXT
from .skill_catalog import SkillRoot, snapshot_catalog_skill


SHA256_PATTERN = re.compile(SHA256_PATTERN_TEXT)
DECISIONS = frozenset({"external_skills", "base_only"})
MODE_VALUES = frozenset({"declared", "inferred", "unsupported"})
MODES = ("plan", "task", "execute")
TOP_FIELDS = frozenset({"schema", "decision", "skills", "selection_sha256"})
SKILL_FIELDS = frozenset(
    {
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
    }
)


def _strict_object(value: object, *, location: str, fields: frozenset[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "expected_object",
            "A JSON object is required.",
            {"location": location},
        )
    missing = sorted(fields - set(value))
    unknown = sorted(set(value) - fields)
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": location, "missing": missing, "unknown": unknown},
        )
    return value


def _text(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise WorkError(
            ExitCode.CONTRACT,
            "empty_text_value",
            "A non-empty string is required.",
            {"location": location},
        )
    return value


def _sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            {"location": location},
        )
    return value


def _root_map(roots: list[SkillRoot]) -> dict[tuple[str, str], SkillRoot]:
    result: dict[tuple[str, str], SkillRoot] = {}
    for root in roots:
        identity = (root.scope, root.locator.replace("\\", "/"))
        if identity in result:
            raise WorkError(
                ExitCode.CLI_USAGE,
                "duplicate_skill_root",
                "Skill root scope and locator pairs must be unique.",
                {"scope": identity[0], "root": identity[1]},
            )
        result[identity] = root
    return result


def selection_sha256(decision: str, skills: list[dict[str, object]]) -> str:
    return canonical_json_sha256({"decision": decision, "skills": skills})


def build_skill_selection(value: object, *, roots: list[SkillRoot]) -> dict[str, object]:
    """Assemble confirmed choices; dependency and inferred-mode decisions stay explicit."""
    request = _strict_object(value, location="skill_selection_request",
                             fields=frozenset({"decision", "skills"}))
    decision = request["decision"]
    if not isinstance(decision, str) or decision not in DECISIONS:
        raise WorkError(ExitCode.CONTRACT, "invalid_skill_selection_decision", "An explicit selection decision is required.")
    choices = request["skills"]
    if not isinstance(choices, list):
        raise WorkError(ExitCode.CONTRACT, "invalid_skill_selection_skills", "Skill choices must be an array.")
    if (decision == "base_only") != (not choices):
        raise WorkError(ExitCode.CONTRACT, "skill_selection_decision_mismatch", "The decision does not match the choices.")
    roots_by_identity = _root_map(roots)
    skills = []
    for index, value in enumerate(choices):
        location = f"skill_selection_request.skills[{index}]"
        fields = {"scope", "root", "source", "recommendation_reason", "dependency_status"}
        if isinstance(value, dict) and "mode_support" in value:
            fields.add("mode_support")
        choice = _strict_object(value, location=location, fields=frozenset(fields))
        for field in fields - {"mode_support"}:
            _text(choice[field], location=f"{location}.{field}")
        if choice["dependency_status"] != "available":
            raise WorkError(ExitCode.CONTRACT, "skill_dependencies_unavailable", "Confirm dependency availability before building a selection.")
        root = roots_by_identity.get((choice["scope"], choice["root"]))
        if root is None:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "selected_skill_root_missing", "The selected skill root is not available.")
        snapshot = snapshot_catalog_skill(root, choice["source"])
        summary, bundle = snapshot["skill"], snapshot["bundle"]
        declared = summary["work_modes"]
        if declared:
            modes = {mode: "declared" if mode in declared else "unsupported" for mode in MODES}
            if "mode_support" in choice and choice["mode_support"] != modes:
                raise WorkError(ExitCode.CONTRACT, "declared_skill_modes_mismatch", "Confirmed modes differ from the declared modes.")
        elif "mode_support" not in choice:
            raise WorkError(ExitCode.CONTRACT, "inferred_skill_modes_required", "Supply confirmed mode support for a skill without declared modes.")
        else:
            modes = choice["mode_support"]
        selected = {field: summary[field] for field in (
            "id", "name", "scope", "root", "source", "description",
            "allow_implicit_invocation", "summary_sha256",
        )}
        selected.update(mode_support=modes, dependency_status=choice["dependency_status"],
                        recommendation_reason=choice["recommendation_reason"], bundle_sha256=bundle["bundle_sha256"])
        skills.append(selected)
    # Resnapshot through the existing validator to reject drift during construction.
    validated = validate_skill_selection({
        "schema": "work-skill-selection/v1", "decision": decision, "skills": skills,
        "selection_sha256": selection_sha256(decision, skills),
    }, roots=roots)
    return validated["skill_selection"]


def validate_skill_selection(value: object, *, roots: list[SkillRoot]) -> dict[str, object]:
    selection = _strict_object(value, location="skill_selection", fields=TOP_FIELDS)
    if selection["schema"] != "work-skill-selection/v1":
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_skill_selection_schema",
            "The skill selection schema is invalid.",
        )
    decision = selection["decision"]
    if decision not in DECISIONS:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_skill_selection_decision",
            "The skill selection decision is invalid.",
            {"decision": decision},
        )
    raw_skills = selection["skills"]
    if not isinstance(raw_skills, list):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_skill_selection_skills",
            "Skill selection skills must be an array.",
        )
    if (decision == "external_skills" and not raw_skills) or (
        decision == "base_only" and raw_skills
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "skill_selection_decision_mismatch",
            "The skill selection decision does not match its skills.",
        )

    roots_by_identity = _root_map(roots)
    skills: list[dict[str, object]] = []
    seen_ids: set[str] = set()
    for index, raw_skill in enumerate(raw_skills):
        location = f"skill_selection.skills[{index}]"
        skill = _strict_object(raw_skill, location=location, fields=SKILL_FIELDS)
        for field in (
            "id",
            "name",
            "scope",
            "root",
            "source",
            "description",
            "recommendation_reason",
        ):
            _text(skill[field], location=f"{location}.{field}")
        _sha256(skill["summary_sha256"], location=f"{location}.summary_sha256")
        _sha256(skill["bundle_sha256"], location=f"{location}.bundle_sha256")
        if not isinstance(skill["allow_implicit_invocation"], bool):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_skill_invocation_policy",
                "allow_implicit_invocation must be a boolean.",
                {"location": f"{location}.allow_implicit_invocation"},
            )
        if skill["dependency_status"] != "available":
            raise WorkError(
                ExitCode.CONTRACT,
                "skill_dependencies_unavailable",
                "A confirmed skill must have available dependencies.",
                {"location": f"{location}.dependency_status"},
            )
        modes = _strict_object(
            skill["mode_support"],
            location=f"{location}.mode_support",
            fields=frozenset(MODES),
        )
        if any(value not in MODE_VALUES for value in modes.values()) or modes["plan"] == "unsupported":
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_skill_mode_support",
                "Confirmed skills must support Plan and use valid mode support values.",
                {"location": f"{location}.mode_support"},
            )
        if skill["id"] in seen_ids:
            raise WorkError(
                ExitCode.CONTRACT,
                "duplicate_selected_skill",
                "A skill may appear only once in a selection.",
                {"id": skill["id"]},
            )
        seen_ids.add(skill["id"])

        root = roots_by_identity.get((skill["scope"], skill["root"]))
        if root is None:
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "selected_skill_root_missing",
                "The selected skill root is not available.",
                {"scope": skill["scope"], "root": skill["root"]},
            )
        current = snapshot_catalog_skill(root, skill["source"])
        current_skill = current["skill"]
        current_bundle = current["bundle"]
        assert isinstance(current_skill, dict)
        assert isinstance(current_bundle, dict)
        expected_values = {
            "id": current_skill["id"],
            "name": current_skill["name"],
            "scope": current_skill["scope"],
            "root": current_skill["root"],
            "source": current_skill["source"],
            "description": current_skill["description"],
            "allow_implicit_invocation": current_skill["allow_implicit_invocation"],
            "summary_sha256": current_skill["summary_sha256"],
            "bundle_sha256": current_bundle["bundle_sha256"],
        }
        if any(skill[field] != expected for field, expected in expected_values.items()):
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "selected_skill_snapshot_mismatch",
                "The selected skill no longer matches its confirmed snapshot.",
                {"id": skill["id"]},
            )
        declared_modes = current_skill["work_modes"]
        assert isinstance(declared_modes, list)
        if declared_modes:
            expected_modes = {
                mode: "declared" if mode in declared_modes else "unsupported"
                for mode in MODES
            }
            if modes != expected_modes:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "declared_skill_modes_mismatch",
                    "Selected mode support does not match declared skill modes.",
                    {"id": skill["id"]},
                )
        elif any(value == "declared" for value in modes.values()):
            raise WorkError(
                ExitCode.CONTRACT,
                "inferred_skill_modes_required",
                "A skill without declared modes cannot use declared mode support.",
                {"id": skill["id"]},
            )
        skills.append(dict(skill))

    stored_hash = _sha256(
        selection["selection_sha256"],
        location="skill_selection.selection_sha256",
    )
    current_hash = selection_sha256(decision, skills)
    if stored_hash != current_hash:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "skill_selection_fingerprint_mismatch",
            "The skill selection fingerprint does not match its contents.",
        )
    return {
        "schema": "work-skill-selection-validation/v1",
        "status": "valid",
        "skill_selection": {
            "schema": "work-skill-selection/v1",
            "decision": decision,
            "skills": skills,
            "selection_sha256": current_hash,
        },
    }


def validate_skill_selection_json(raw: bytes, *, roots: list[SkillRoot]) -> dict[str, object]:
    try:
        value = json.loads(decode_utf8(raw, source="stdin"))
    except json.JSONDecodeError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_json",
            "The input is not valid JSON.",
            {"line": error.lineno, "column": error.colno},
        ) from error
    return validate_skill_selection(value, roots=roots)
