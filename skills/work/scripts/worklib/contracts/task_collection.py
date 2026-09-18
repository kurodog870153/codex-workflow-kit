from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from ..models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, read_raw
from ..foundation.markdown import parse_json_contract, render_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..services.skill_catalog import SkillRoot
from ..services.plan_validation import validate_plan_contract
from .task_collection_semantics import render_task_contract, validate_task_contract
from .task_index import validate_task_index_contract
from .task_item import validate_task_item_contract
from .task_collection_models import (
    TaskCollectionFingerprintContract, TaskCollectionProjectionContract,
    TaskCollectionValidationContract,
)


def _semantic_changes(value: object) -> object:
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


def _semantic_projection(
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
    contract["schema"] = "work-task-collection-projection/v1"
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
        contract["changes"] = _semantic_changes(index["changes"])
    return contract


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

    semantic_contract = TaskCollectionProjectionContract.model_validate(_semantic_projection(
        index,
        items,
        task_path=actual_index_path,
        source_plan_sha256=actual_plan_sha,
    )).to_canonical_dict()
    semantic_validation = validate_task_contract(
        render_task_contract(semantic_contract),
        source="TASK collection semantic projection",
        actual_task_path=actual_index_path,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=validate_file_state,
        skill_roots=skill_roots,
        _source_plan_raw=source_plan_raw,
    )

    fingerprint = TaskCollectionFingerprintContract.model_validate({
        "schema": "work-task-collection-fingerprint/v1",
        "task_index_sha256": index_validation["task_index_sha256"],
        "items": [
            {"id": task_id, "task_item_sha256": item_sha256[task_id]}
            for task_id in expected_ids
        ],
    }).to_canonical_dict()
    collection_sha256 = hashlib.sha256(render_json_contract(fingerprint)).hexdigest()
    return TaskCollectionValidationContract.model_validate({
        "schema": "work-task-collection-validation/v1",
        "requirement_id": index["requirement_id"],
        "spec_id": index["spec_id"],
        "task_ids": expected_ids,
        "task_count": len(expected_ids),
        "task_index_sha256": index_validation["task_index_sha256"],
        "task_item_sha256": item_sha256,
        "task_collection_sha256": collection_sha256,
        "source_plan_sha256": actual_plan_sha,
        "instructions_sha256": semantic_validation["instructions_sha256"],
        "task_instructions_sha256": semantic_validation["task_instructions_sha256"],
        "task_skill_ids": semantic_validation["task_skill_ids"],
        "hierarchy_selection_sha256": plan_validation["hierarchy_selection_sha256"],
        "skill_selection_sha256": plan_validation["skill_selection_sha256"],
        "collection_contract": semantic_contract,
    }).to_canonical_dict()
