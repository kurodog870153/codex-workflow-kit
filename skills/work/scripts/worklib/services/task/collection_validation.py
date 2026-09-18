from __future__ import annotations

import hashlib
from typing import Any

from ...technical.infrastructure.json_contract import render_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.task_collection import (
    TaskCollectionFingerprintContract,
    TaskCollectionProjectionContract,
    TaskCollectionValidationContract,
)


def semantic_projection(index: dict[str, Any], items: list[dict[str, Any]], *,
                        task_path: str, source_plan_sha256: str) -> dict[str, Any]:
    contract = {key: value for key, value in index.items() if key not in {"schema", "tasks", "changes"}}
    contract["schema"] = "work-task-collection-projection/v1"
    contract["artifacts"] = {**index["artifacts"], "task": task_path}
    contract["source_plan"] = {**index["source_plan"], "canonical_sha256": source_plan_sha256}
    contract["tasks"] = [{key: value for key, value in item.items() if key != "schema"} for item in items]
    if "changes" in index:
        changes: list[object] = []
        for raw_change in index["changes"]:
            if not isinstance(raw_change, dict):
                changes.append(raw_change)
                continue
            change = dict(raw_change)
            edits = change.get("edits")
            if isinstance(edits, list):
                change["edits"] = [
                    {key: item[key] for key in ("operation", "path", "before", "after") if isinstance(item, dict) and key in item}
                    if isinstance(item, dict) else item for item in edits
                ]
            changes.append(change)
        contract["changes"] = changes
    return TaskCollectionProjectionContract.model_validate(contract).to_canonical_dict()


def build_collection_validation(*, index: dict[str, Any], index_validation: dict[str, object],
                                item_validations: dict[str, dict[str, object]],
                                semantic_contract: dict[str, Any], semantic_validation: dict[str, object],
                                plan_validation: dict[str, object], actual_plan_sha256: str) -> dict[str, object]:
    expected_ids = index_validation["task_ids"]
    assert isinstance(expected_ids, list)
    item_sha256 = {task_id: item_validations[task_id]["task_item_sha256"] for task_id in expected_ids}
    fingerprint = TaskCollectionFingerprintContract.model_validate({
        "schema": "work-task-collection-fingerprint/v1",
        "task_index_sha256": index_validation["task_index_sha256"],
        "items": [{"id": task_id, "task_item_sha256": item_sha256[task_id]} for task_id in expected_ids],
    }).to_canonical_dict()
    collection_sha256 = hashlib.sha256(render_json_contract(fingerprint)).hexdigest()
    return TaskCollectionValidationContract.model_validate({
        "schema": "work-task-collection-validation/v1", "requirement_id": index["requirement_id"],
        "spec_id": index["spec_id"], "task_ids": expected_ids, "task_count": len(expected_ids),
        "task_index_sha256": index_validation["task_index_sha256"], "task_item_sha256": item_sha256,
        "task_collection_sha256": collection_sha256, "source_plan_sha256": actual_plan_sha256,
        "instructions_sha256": semantic_validation["instructions_sha256"],
        "task_instructions_sha256": semantic_validation["task_instructions_sha256"],
        "task_skill_ids": semantic_validation["task_skill_ids"],
        "hierarchy_selection_sha256": plan_validation["hierarchy_selection_sha256"],
        "skill_selection_sha256": plan_validation["skill_selection_sha256"],
        "collection_contract": semantic_contract,
    }).to_canonical_dict()


def collection_fingerprint_sha256(
    task_index_sha256: str,
    references: list[dict[str, Any]],
) -> str:
    fingerprint = TaskCollectionFingerprintContract.model_validate({
        "schema": "work-task-collection-fingerprint/v1",
        "task_index_sha256": task_index_sha256,
        "items": [
            {"id": reference["id"], "task_item_sha256": reference["canonical_sha256"]}
            for reference in references
        ],
    }).to_canonical_dict()
    return hashlib.sha256(render_json_contract(fingerprint)).hexdigest()


def require_item_set(item_raw: dict[str, bytes], expected_ids: list[str]) -> None:
    if set(item_raw) != set(expected_ids):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_collection_item_set_mismatch",
                        "The supplied TASK item set does not match the formal index.",
                        {"missing": sorted(set(expected_ids) - set(item_raw)),
                         "unknown": sorted(set(item_raw) - set(expected_ids))})


__all__ = ["build_collection_validation", "collection_fingerprint_sha256", "require_item_set", "semantic_projection"]
