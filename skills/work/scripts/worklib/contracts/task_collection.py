from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, read_raw
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot
from .plan import render_plan_contract, validate_plan_contract
from .task import render_task_contract, validate_task_contract
from .task_index import validate_task_index_contract
from .task_item import validate_task_item_contract


def _legacy_changes(value: object) -> object:
    if not isinstance(value, list):
        return value
    result: list[object] = []
    for raw_change in value:
        if not isinstance(raw_change, dict):
            result.append(raw_change)
            continue
        change = dict(raw_change)
        edits = change.get("edits")
        if isinstance(edits, list):
            change["edits"] = [
                {
                    key: item[key]
                    for key in ("operation", "path", "before", "after")
                    if isinstance(item, dict) and key in item
                }
                if isinstance(item, dict)
                else item
                for item in edits
            ]
        result.append(change)
    return result


def _logical_v1_contract(
    index: dict[str, Any],
    items: list[dict[str, Any]],
    *,
    task_path: str,
    source_plan_sha256: str,
) -> dict[str, Any]:
    contract = {
        key: value
        for key, value in index.items()
        if key not in {"schema", "tasks", "changes"}
    }
    contract["schema"] = "work-task/v1"
    contract["artifacts"] = {**index["artifacts"], "task": task_path}
    contract["source_plan"] = {
        **index["source_plan"],
        "canonical_sha256": source_plan_sha256,
    }
    contract["tasks"] = [
        {key: value for key, value in item.items() if key != "schema"}
        for item in items
    ]
    if "changes" in index:
        contract["changes"] = _legacy_changes(index["changes"])
    return contract


def _shadow_plan(
    raw: bytes,
    *,
    source: str,
    task_path: str,
) -> tuple[dict[str, Any], bytes]:
    plan = parse_json_contract(raw, source=source)
    artifacts = plan.get("artifacts")
    if not isinstance(artifacts, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_artifact_paths",
            "The Plan artifacts must be an object.",
        )
    shadow = {**plan, "artifacts": {**artifacts, "task": task_path}}
    return shadow, render_plan_contract(shadow)


def validate_task_collection_contract(
    index_raw: bytes,
    item_raw: dict[str, bytes],
    *,
    source: str,
    actual_index_path: str,
    project_root: Path,
    user_config_root: str,
    validate_file_state: bool = True,
    skill_roots: list[SkillRoot] | None = None,
    _source_plan_raw: bytes | None = None,
) -> dict[str, object]:
    index_validation = validate_task_index_contract(
        index_raw,
        source=source,
        actual_index_path=actual_index_path,
        project_root=project_root,
    )
    index = parse_json_contract(index_raw, source=source)
    expected_ids = index_validation["task_ids"]
    assert isinstance(expected_ids, list)
    if set(item_raw) != set(expected_ids):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_collection_item_set_mismatch",
            "The supplied TASK item set does not match the formal index.",
            {
                "missing": sorted(set(expected_ids) - set(item_raw)),
                "unknown": sorted(set(item_raw) - set(expected_ids)),
            },
        )

    items: list[dict[str, Any]] = []
    item_sha256: dict[str, str] = {}
    for reference in index["tasks"]:
        task_id = reference["id"]
        raw = item_raw[task_id]
        validation = validate_task_item_contract(
            raw,
            source=f"{source} {reference['path']}",
            expected_task_id=task_id,
        )
        actual_sha = validation["task_item_sha256"]
        if actual_sha != reference["canonical_sha256"]:
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "task_item_fingerprint_mismatch",
                "A TASK item fingerprint does not match the formal index.",
                {"task_id": task_id, "expected": reference["canonical_sha256"], "actual": actual_sha},
            )
        item_sha256[task_id] = actual_sha
        items.append(parse_json_contract(raw, source=f"{source} {reference['path']}"))

    artifacts = index["artifacts"]
    if _source_plan_raw is None:
        _, plan_path = resolve_project_relative_path(
            project_root, artifacts["plan"], field="plan_path"
        )
        source_plan_raw = read_raw(plan_path)
    else:
        source_plan_raw = _source_plan_raw
    plan_validation = validate_plan_contract(
        source_plan_raw,
        source="TASK collection source Plan",
        actual_plan_path=artifacts["plan"],
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
        _allow_task_index=True,
    )
    actual_plan_sha = canonical_sha256(source_plan_raw, source="TASK collection source Plan")
    if index["source_plan"]["canonical_sha256"] != actual_plan_sha:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "source_plan_fingerprint_mismatch",
            "The TASK collection source Plan fingerprint does not match the validated Plan.",
        )
    if index["source_plan"]["hierarchy_selection_sha256"] != plan_validation["hierarchy_selection_sha256"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "source_plan_hierarchy_selection_mismatch",
            "The TASK collection hierarchy selection fingerprint does not match the Plan.",
        )
    plan = parse_json_contract(source_plan_raw, source="TASK collection source Plan")
    if plan["requirement_id"] != index["requirement_id"] or plan["artifacts"] != artifacts:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "source_plan_identity_mismatch",
            "The TASK collection identity or artifacts do not match the source Plan.",
        )

    collection_directory = actual_index_path.rsplit("/", 1)[0]
    legacy_task_path = f"{collection_directory}/task.json"
    _, shadow_plan_raw = _shadow_plan(
        source_plan_raw,
        source="TASK collection source Plan",
        task_path=legacy_task_path,
    )
    shadow_plan_sha = canonical_sha256(shadow_plan_raw, source="logical v1 source Plan")
    logical_contract = _logical_v1_contract(
        index,
        items,
        task_path=legacy_task_path,
        source_plan_sha256=shadow_plan_sha,
    )
    logical_validation = validate_task_contract(
        render_task_contract(logical_contract),
        source="logical v1 TASK collection",
        actual_task_path=legacy_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=validate_file_state,
        skill_roots=skill_roots,
        _source_plan_raw=shadow_plan_raw,
    )

    fingerprint = {
        "schema": "work-task-collection-fingerprint/v2",
        "task_index_sha256": index_validation["task_index_sha256"],
        "items": [
            {"id": task_id, "task_item_sha256": item_sha256[task_id]}
            for task_id in expected_ids
        ],
    }
    collection_sha256 = hashlib.sha256(render_json_contract(fingerprint)).hexdigest()
    return {
        "schema": "work-task-collection-validation/v2",
        "requirement_id": index["requirement_id"],
        "spec_id": index["spec_id"],
        "task_ids": expected_ids,
        "task_count": len(expected_ids),
        "task_index_sha256": index_validation["task_index_sha256"],
        "task_item_sha256": item_sha256,
        "task_collection_sha256": collection_sha256,
        "source_plan_sha256": actual_plan_sha,
        "instructions_sha256": logical_validation["instructions_sha256"],
        "task_instructions_sha256": logical_validation["task_instructions_sha256"],
        "task_skill_ids": logical_validation["task_skill_ids"],
        "hierarchy_selection_sha256": plan_validation["hierarchy_selection_sha256"],
        "skill_selection_sha256": plan_validation["skill_selection_sha256"],
        "logical_contract": logical_contract,
    }
