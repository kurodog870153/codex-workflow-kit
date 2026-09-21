from __future__ import annotations

from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.skill import SkillRoot
from ...services.task.collection_validation import collection_fingerprint_sha256
from ...services.task.document import parse_task_contract
from ...services.task.storage import (
    read_project_task_source,
    read_task_index,
    read_task_item,
    task_collection_item_names,
)
from .collection import validate_task_collection_contract
from .index import validate_task_index_contract
from .item import validate_task_item_contract
from .plan import validate_task_source_plan


def _index(project_root: Path, raw_index_path: str):
    normalized, raw, contract = read_task_index(project_root, raw_index_path)
    validation = validate_task_index_contract(
        raw, source=raw_index_path, actual_index_path=normalized,
        project_root=project_root,
    )
    if validation["requirement_id"] != contract.get("requirement_id"):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_index_identity_mismatch",
                        "The TASK index identity changed during validation.")
    return raw, contract, validation


def _item(project_root: Path, raw_index_path: str, reference: dict[str, Any], *, requirement_id: str):
    path, raw, contract = read_task_item(
        project_root, raw_index_path, reference, requirement_id=requirement_id
    )
    validation = validate_task_item_contract(
        raw, source=str(path), expected_task_id=reference["id"]
    )
    if validation["task_item_sha256"] != reference["canonical_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_item_fingerprint_mismatch",
                        "A TASK item fingerprint does not match the formal index.",
                        {"task_id": reference["id"]})
    return raw, contract, validation


def _all_items(project_root: Path, raw_index_path: str, index: dict[str, Any]):
    raw_items: dict[str, bytes] = {}
    items: dict[str, dict[str, Any]] = {}
    for reference in index["tasks"]:
        raw, item, _ = _item(project_root, raw_index_path, reference,
                             requirement_id=index["requirement_id"])
        raw_items[reference["id"]] = raw
        items[reference["id"]] = item
    return raw_items, items


def _reject_orphans(project_root: Path, raw_index_path: str, index: dict[str, Any]) -> None:
    expected = {reference["path"].split("/", 1)[1] for reference in index["tasks"]}
    observed = task_collection_item_names(project_root, raw_index_path)
    if observed != expected:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_collection_directory_mismatch",
                        "The TASK item directory does not exactly match the formal index.",
                        {"missing": sorted(expected - observed), "orphan": sorted(observed - expected)})


def load_task_closure(project_root: Path, raw_index_path: str, task_id: str) -> dict[str, dict[str, Any]]:
    _, index, _ = _index(project_root, raw_index_path)
    references = {item["id"]: item for item in index["tasks"]}
    if task_id not in references:
        raise WorkError(ExitCode.CONTRACT, "unknown_task_id", "The selected TASK does not exist.", {"task_id": task_id})
    selected: set[str] = set()
    items: dict[str, dict[str, Any]] = {}
    def visit(current: str) -> None:
        if current in selected:
            return
        reference = references.get(current)
        if reference is None:
            raise WorkError(ExitCode.CONTRACT, "invalid_task_dependency", "A TASK dependency is unknown.", {"task_id": current})
        selected.add(current)
        _, item, _ = _item(project_root, raw_index_path, reference,
                           requirement_id=index["requirement_id"])
        items[current] = item
        for dependency in item.get("dependencies", []):
            visit(dependency)
    visit(task_id)
    return {reference["id"]: items[reference["id"]] for reference in index["tasks"] if reference["id"] in selected}


def load_task_execution_context(project_root: Path, user_config_root: str,
                                raw_task_path: str, task_id: str, *,
                                skill_roots: list[SkillRoot] | None = None) -> dict[str, object]:
    index_raw, index, index_validation = _index(project_root, raw_task_path)
    _, plan_path, plan_raw = read_project_task_source(
        project_root, index["artifacts"]["plan"], field="plan_path"
    )
    plan_validation = validate_task_source_plan(
        plan_raw, source=str(plan_path), actual_plan_path=index["artifacts"]["plan"],
        project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots, _allow_task_index=True,
    )
    if index["source_plan"]["canonical_sha256"] != plan_validation["plan_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "source_plan_fingerprint_mismatch",
                        "The TASK collection source Plan fingerprint does not match the validated Plan.")
    if index["source_plan"]["hierarchy_selection_sha256"] != plan_validation["hierarchy_selection_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "source_plan_hierarchy_selection_mismatch",
                        "The TASK collection hierarchy selection fingerprint does not match the Plan.")
    references = {item["id"]: item for item in index["tasks"]}
    if task_id not in references:
        raise WorkError(ExitCode.CONTRACT, "unknown_task_id", "The selected TASK does not exist.", {"task_id": task_id})
    items: dict[str, dict[str, Any]] = {}
    item_sha256: dict[str, str] = {}
    execution_ids: set[str] = set()
    sources: dict[str, bytes] = {raw_task_path: index_raw}
    def load(current: str) -> dict[str, Any]:
        if current in items:
            return items[current]
        reference = references.get(current)
        if reference is None:
            raise WorkError(ExitCode.CONTRACT, "invalid_task_dependency", "A TASK dependency is unknown.", {"task_id": current})
        raw, item, validation = _item(project_root, raw_task_path, reference,
                                      requirement_id=index["requirement_id"])
        items[current] = item
        item_sha256[current] = validation["task_item_sha256"]
        sources[f"{raw_task_path.rsplit('/', 1)[0]}/{reference['path']}"] = raw
        return item
    def visit(current: str) -> None:
        if current in execution_ids:
            return
        execution_ids.add(current)
        for dependency in load(current).get("dependencies", []):
            visit(dependency)
    visit(task_id)
    for reference in index["tasks"]:
        load(reference["id"])
    collection_sha256 = collection_fingerprint_sha256(
        index_validation["task_index_sha256"], index["tasks"]
    )
    contract = {key: value for key, value in index.items() if key not in {"schema", "tasks", "changes"}}
    contract["schema"] = "work-task-execution-view/v1"
    contract["tasks"] = [{key: value for key, value in items[reference["id"]].items() if key != "schema"}
                         for reference in index["tasks"] if reference["id"] in execution_ids]
    validation = {"schema": "work-task-execution-validation/v1",
        "requirement_id": index["requirement_id"], "spec_id": index["spec_id"],
        "task_ids": [reference["id"] for reference in index["tasks"]],
        "task_collection_sha256": collection_sha256,
        "task_index_sha256": index_validation["task_index_sha256"],
        "task_item_sha256": item_sha256,
        "instructions_sha256": index["instruction_selection"]["instructions_sha256"],
        "task_instructions_sha256": {current: item["instruction_selection"]["instructions_sha256"] for current, item in items.items()},
        "task_skill_ids": {current: item["skill_id"] for current, item in items.items()},
        "hierarchy_selection_sha256": index["source_plan"]["hierarchy_selection_sha256"]}
    return {"contract": contract, "validation": validation, "sources": sources}


def load_task_collection(project_root: Path, user_config_root: str, raw_index_path: str, *,
                         validate_file_state: bool = True,
                         skill_roots: list[SkillRoot] | None = None,
                         raw: bytes | None = None) -> dict[str, object]:
    if raw is None:
        index_raw, index, _ = _index(project_root, raw_index_path)
    else:
        validate_task_index_contract(raw, source=raw_index_path,
            actual_index_path=raw_index_path, project_root=project_root)
        index_raw, index = raw, parse_task_contract(raw, source=raw_index_path)
    raw_items, _ = _all_items(project_root, raw_index_path, index)
    _reject_orphans(project_root, raw_index_path, index)
    return validate_task_collection_contract(index_raw, raw_items, source=raw_index_path,
        actual_index_path=raw_index_path, project_root=project_root,
        user_config_root=user_config_root, validate_file_state=validate_file_state,
        skill_roots=skill_roots)


__all__ = ["load_task_closure", "load_task_collection", "load_task_execution_context"]
