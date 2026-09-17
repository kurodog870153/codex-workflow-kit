from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from ..contracts.task import validate_task_file
from ..contracts.task_collection import validate_task_collection_contract
from ..contracts.task_index import validate_task_index_contract
from ..contracts.task_item import validate_task_item_contract
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.markdown import render_json_contract
from ..foundation.paths import (
    resolve_project_relative_path,
    resolve_task_collection_item_path,
)
from ..skills.catalog import SkillRoot


def task_artifact_format(raw_path: str) -> str:
    name = Path(raw_path.replace("\\", "/")).name
    if name == "task.json":
        return "v1"
    if name == "index.json":
        return "v2"
    raise WorkError(
        ExitCode.CONTRACT,
        "unsupported_task_artifact_entry",
        "A formal TASK entry must be task.json or index.json.",
        {"path": raw_path},
    )


def load_task_index(
    project_root: Path,
    raw_index_path: str,
) -> tuple[bytes, dict[str, Any], dict[str, object]]:
    normalized, index_path = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    raw = read_raw(index_path)
    contract = parse_json_contract(raw, source=str(index_path))
    requirement_id = contract.get("requirement_id")
    validation = validate_task_index_contract(
        raw,
        source=str(index_path),
        actual_index_path=normalized,
        project_root=project_root,
    )
    if validation["requirement_id"] != requirement_id:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "task_index_identity_mismatch", "The TASK index identity changed during validation.")
    return raw, contract, validation


def load_task_item(
    project_root: Path,
    raw_index_path: str,
    reference: dict[str, Any],
    *,
    requirement_id: str,
) -> tuple[bytes, dict[str, Any], dict[str, object]]:
    task_id = reference["id"]
    _, path = resolve_task_collection_item_path(
        project_root,
        requirement_id,
        raw_index_path,
        task_id,
        reference["path"],
    )
    raw = read_raw(path)
    validation = validate_task_item_contract(
        raw, source=str(path), expected_task_id=task_id
    )
    if validation["task_item_sha256"] != reference["canonical_sha256"]:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_item_fingerprint_mismatch",
            "A TASK item fingerprint does not match the formal index.",
            {"task_id": task_id},
        )
    return raw, parse_json_contract(raw, source=str(path)), validation


def _load_all_items(
    project_root: Path,
    raw_index_path: str,
    index: dict[str, Any],
) -> tuple[dict[str, bytes], dict[str, dict[str, Any]]]:
    raw_items: dict[str, bytes] = {}
    items: dict[str, dict[str, Any]] = {}
    for reference in index["tasks"]:
        raw, item, _ = load_task_item(
            project_root,
            raw_index_path,
            reference,
            requirement_id=index["requirement_id"],
        )
        raw_items[reference["id"]] = raw
        items[reference["id"]] = item
    return raw_items, items


def _reject_orphan_items(
    project_root: Path,
    raw_index_path: str,
    index: dict[str, Any],
) -> None:
    _, index_path = resolve_project_relative_path(
        project_root, raw_index_path, field="task_index_path"
    )
    directory = index_path.parent / "tasks"
    expected = {reference["path"].split("/", 1)[1] for reference in index["tasks"]}
    try:
        observed = {
            path.name
            for path in directory.iterdir()
            if path.suffix == ".json" and path.is_file()
        } if directory.is_dir() else set()
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "task_collection_directory_read_failed",
            "The TASK item directory could not be inspected.",
            {"path": str(directory)},
        ) from error
    if observed != expected:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_collection_directory_mismatch",
            "The TASK item directory does not exactly match the formal index.",
            {"missing": sorted(expected - observed), "orphan": sorted(observed - expected)},
        )


def load_task_headers(
    project_root: Path,
    raw_index_path: str,
) -> dict[str, dict[str, object]]:
    _, index, _ = load_task_index(project_root, raw_index_path)
    _, items = _load_all_items(project_root, raw_index_path, index)
    return {
        task_id: {
            "id": task_id,
            "dependencies": list(item.get("dependencies", [])),
            "task_item_sha256": reference["canonical_sha256"],
        }
        for reference in index["tasks"]
        for task_id, item in [(reference["id"], items[reference["id"]])]
    }


def load_task_closure(
    project_root: Path,
    raw_index_path: str,
    task_id: str,
) -> dict[str, dict[str, Any]]:
    _, index, _ = load_task_index(project_root, raw_index_path)
    references = {item["id"]: item for item in index["tasks"]}
    if task_id not in references:
        raise WorkError(ExitCode.CONTRACT, "unknown_task_id", "The selected TASK does not exist.", {"task_id": task_id})
    selected: set[str] = set()
    items: dict[str, dict[str, Any]] = {}

    def visit(current: str) -> None:
        if current in selected:
            return
        if current not in references:
            raise WorkError(ExitCode.CONTRACT, "invalid_task_dependency", "A TASK dependency is unknown.", {"task_id": current})
        selected.add(current)
        _, item, _ = load_task_item(
            project_root,
            raw_index_path,
            references[current],
            requirement_id=index["requirement_id"],
        )
        items[current] = item
        for dependency in item.get("dependencies", []):
            visit(dependency)

    visit(task_id)
    return {
        reference["id"]: items[reference["id"]]
        for reference in index["tasks"]
        if reference["id"] in selected
    }


def load_task_execution_context(
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    task_id: str,
    *,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    if task_artifact_format(raw_task_path) == "v1":
        normalized, path = resolve_project_relative_path(
            project_root, raw_task_path, field="task_path"
        )
        raw = read_raw(path)
        validation = validate_task_file(
            project_root,
            user_config_root,
            normalized,
            skill_roots=skill_roots,
        )
        return {
            "contract": parse_json_contract(raw, source=str(path)),
            "validation": validation,
            "sources": {normalized: raw},
        }

    index_raw, index, index_validation = load_task_index(
        project_root, raw_task_path
    )
    references = {item["id"]: item for item in index["tasks"]}
    if task_id not in references:
        raise WorkError(
            ExitCode.CONTRACT,
            "unknown_task_id",
            "The selected TASK does not exist.",
            {"task_id": task_id},
        )
    items: dict[str, dict[str, Any]] = {}
    item_sha256: dict[str, str] = {}
    sources: dict[str, bytes] = {raw_task_path: index_raw}

    def visit(current: str) -> None:
        if current in items:
            return
        reference = references.get(current)
        if reference is None:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_task_dependency",
                "A TASK dependency is unknown.",
                {"task_id": current},
            )
        raw, item, validation = load_task_item(
            project_root,
            raw_task_path,
            reference,
            requirement_id=index["requirement_id"],
        )
        items[current] = item
        item_sha256[current] = validation["task_item_sha256"]
        directory = raw_task_path.rsplit("/", 1)[0]
        sources[f"{directory}/{reference['path']}"] = raw
        for dependency in item.get("dependencies", []):
            visit(dependency)

    visit(task_id)
    fingerprint = {
        "schema": "work-task-collection-fingerprint/v2",
        "task_index_sha256": index_validation["task_index_sha256"],
        "items": [
            {
                "id": reference["id"],
                "task_item_sha256": reference["canonical_sha256"],
            }
            for reference in index["tasks"]
        ],
    }
    collection_sha256 = hashlib.sha256(
        render_json_contract(fingerprint)
    ).hexdigest()
    contract = {
        key: value
        for key, value in index.items()
        if key not in {"schema", "tasks", "changes"}
    }
    contract["schema"] = "work-task-execution-view/v2"
    contract["tasks"] = [
        {key: value for key, value in items[reference["id"]].items() if key != "schema"}
        for reference in index["tasks"]
        if reference["id"] in items
    ]
    task_instructions = {
        current: item["instruction_selection"]["instructions_sha256"]
        for current, item in items.items()
    }
    validation = {
        "schema": "work-task-execution-validation/v2",
        "requirement_id": index["requirement_id"],
        "spec_id": index["spec_id"],
        "task_ids": [reference["id"] for reference in index["tasks"]],
        "task_collection_sha256": collection_sha256,
        "task_index_sha256": index_validation["task_index_sha256"],
        "task_item_sha256": item_sha256,
        "instructions_sha256": index["instruction_selection"]["instructions_sha256"],
        "task_instructions_sha256": task_instructions,
        "task_skill_ids": {
            current: item["skill_id"] for current, item in items.items()
        },
        "hierarchy_selection_sha256": index["source_plan"]["hierarchy_selection_sha256"],
    }
    return {"contract": contract, "validation": validation, "sources": sources}


def load_task_collection(
    project_root: Path,
    user_config_root: str,
    raw_index_path: str,
    *,
    validate_file_state: bool = True,
    skill_roots: list[SkillRoot] | None = None,
    _index_raw: bytes | None = None,
) -> dict[str, object]:
    if _index_raw is None:
        index_raw, index, _ = load_task_index(project_root, raw_index_path)
    else:
        normalized, _ = resolve_project_relative_path(
            project_root, raw_index_path, field="task_index_path"
        )
        validate_task_index_contract(
            _index_raw,
            source=raw_index_path,
            actual_index_path=normalized,
            project_root=project_root,
        )
        index_raw = _index_raw
        index = parse_json_contract(index_raw, source=raw_index_path)
    raw_items, _ = _load_all_items(project_root, raw_index_path, index)
    _reject_orphan_items(project_root, raw_index_path, index)
    return validate_task_collection_contract(
        index_raw,
        raw_items,
        source=raw_index_path,
        actual_index_path=raw_index_path,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=validate_file_state,
        skill_roots=skill_roots,
    )


def load_task_artifact(
    project_root: Path,
    user_config_root: str,
    raw_path: str,
    *,
    validate_file_state: bool = True,
    skill_roots: list[SkillRoot] | None = None,
    raw: bytes | None = None,
) -> dict[str, object]:
    if task_artifact_format(raw_path) == "v1":
        if raw is not None:
            from ..contracts.task import validate_task_contract

            return validate_task_contract(
                raw,
                source=raw_path,
                actual_task_path=raw_path,
                project_root=project_root,
                user_config_root=user_config_root,
                validate_file_state=validate_file_state,
                skill_roots=skill_roots,
            )
        return validate_task_file(
            project_root, user_config_root, raw_path, skill_roots=skill_roots
        )
    return load_task_collection(
        project_root,
        user_config_root,
        raw_path,
        validate_file_state=validate_file_state,
        skill_roots=skill_roots,
        _index_raw=raw,
    )
