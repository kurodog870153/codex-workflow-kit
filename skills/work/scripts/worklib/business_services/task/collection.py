from __future__ import annotations

from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.skill import SkillRoot
from ...services.task.collection_validation import (
    build_collection_validation,
    require_item_set,
    semantic_projection,
)
from ...services.task.document import parse_task_contract
from ...services.task.storage import read_project_task_source
from .document import render_task_contract
from .index import validate_task_index_contract
from .item import validate_task_item_contract
from .plan import validate_task_source_plan
from .semantic import validate_task_contract


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
        index_raw, source=source, actual_index_path=actual_index_path,
        project_root=project_root,
    )
    index = parse_task_contract(index_raw, source=source)
    expected_ids = index_validation["task_ids"]
    assert isinstance(expected_ids, list)
    require_item_set(item_raw, expected_ids)
    items: list[dict[str, Any]] = []
    item_validations: dict[str, dict[str, object]] = {}
    for reference in index["tasks"]:
        task_id = reference["id"]
        raw = item_raw[task_id]
        validation = validate_task_item_contract(
            raw, source=f"{source} {reference['path']}", expected_task_id=task_id
        )
        actual_sha = validation["task_item_sha256"]
        if actual_sha != reference["canonical_sha256"]:
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY, "task_item_fingerprint_mismatch",
                "A TASK item fingerprint does not match the formal index.",
                {"task_id": task_id, "expected": reference["canonical_sha256"], "actual": actual_sha},
            )
        item_validations[task_id] = validation
        items.append(parse_task_contract(raw, source=f"{source} {reference['path']}"))
    artifacts = index["artifacts"]
    if _source_plan_raw is None:
        _, _, plan_raw = read_project_task_source(
            project_root, artifacts["plan"], field="plan_path"
        )
    else:
        plan_raw = _source_plan_raw
    plan_validation = validate_task_source_plan(
        plan_raw, source="TASK collection source Plan",
        actual_plan_path=artifacts["plan"], project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
        _allow_task_index=True,
    )
    actual_plan_sha = plan_validation["plan_sha256"]
    if index["source_plan"]["canonical_sha256"] != actual_plan_sha:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "source_plan_fingerprint_mismatch",
                        "The TASK collection source Plan fingerprint does not match the validated Plan.")
    if index["source_plan"]["hierarchy_selection_sha256"] != plan_validation["hierarchy_selection_sha256"]:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "source_plan_hierarchy_selection_mismatch",
                        "The TASK collection hierarchy selection fingerprint does not match the Plan.")
    plan = parse_task_contract(plan_raw, source="TASK collection source Plan")
    if plan["requirement_id"] != index["requirement_id"] or plan["artifacts"] != artifacts:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "source_plan_identity_mismatch",
                        "The TASK collection identity or artifacts do not match the source Plan.")
    semantic_contract = semantic_projection(
        index, items, task_path=actual_index_path,
        source_plan_sha256=str(actual_plan_sha),
    )
    semantic_validation = validate_task_contract(
        render_task_contract(semantic_contract),
        source="TASK collection semantic projection",
        actual_task_path=actual_index_path,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=validate_file_state,
        skill_roots=skill_roots,
        _source_plan_raw=plan_raw,
    )
    return build_collection_validation(
        index=index, index_validation=index_validation,
        item_validations=item_validations, semantic_contract=semantic_contract,
        semantic_validation=semantic_validation, plan_validation=plan_validation,
        actual_plan_sha256=str(actual_plan_sha),
    )


__all__ = ["validate_task_collection_contract"]
