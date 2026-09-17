from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ..contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from ..contracts.task import (
    prepare_task_json_contract,
    validate_task_file,
)
from ..contracts.task_collection import validate_task_collection_contract
from ..contracts.task_index import render_task_index_contract
from ..contracts.task_item import render_task_item_contract, validate_task_item_contract
from ..contracts.plan import validate_plan_contract
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import resolve_project_relative_path
from ..foundation.paths import validate_task_collection_index_path
from ..skills.catalog import SkillRoot
from .task_collection import load_task_collection


def prepare_task_collection_create(
    raw: bytes, *, source: str, raw_plan_path: str, raw_task_path: str,
    project_root: Path, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    """Build and fully validate the exact v2 index/item byte set in memory."""
    contract = parse_json_contract(raw, source=source)
    if contract.get("schema") != "work-task/v1":
        raise WorkError(ExitCode.CONTRACT, "task_create_schema", "V2 create input must be a complete logical work-task/v1 contract.")
    artifacts = contract.get("artifacts")
    if not isinstance(artifacts, dict) or artifacts.get("task") != raw_task_path or artifacts.get("plan") != raw_plan_path:
        raise WorkError(ExitCode.CONTRACT, "task_create_path_mismatch", "The explicit create paths must match the TASK artifact paths.")
    normalized_index, _ = validate_task_collection_index_path(
        project_root, contract.get("requirement_id"), raw_task_path
    )
    _, plan_path = resolve_project_relative_path(project_root, raw_plan_path, field="plan_path")
    plan_raw = read_raw(plan_path)
    validate_plan_contract(
        plan_raw, source=str(plan_path), actual_plan_path=raw_plan_path,
        project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots, _allow_task_index=True,
    )
    raw_tasks = contract.get("tasks")
    if not isinstance(raw_tasks, list) or not raw_tasks:
        raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "TASK create requires non-empty tasks.")
    items: dict[str, bytes] = {}
    references = []
    for row in raw_tasks:
        if not isinstance(row, dict) or not isinstance(row.get("id"), str):
            raise WorkError(ExitCode.CONTRACT, "invalid_task_item", "Each TASK create row must be an object with an ID.")
        task_id = row["id"]
        item_raw = render_task_item_contract({"schema": "work-task-item/v2", **row})
        checked = validate_task_item_contract(item_raw, source=f"{source} {task_id}", expected_task_id=task_id)
        items[task_id] = item_raw
        references.append({"id": task_id, "path": f"tasks/{task_id}.json", "canonical_sha256": checked["task_item_sha256"]})
    index = {key: value for key, value in contract.items() if key not in {"schema", "tasks"}}
    index["schema"] = "work-task-index/v2"
    index["tasks"] = references
    index_raw = render_task_index_contract(index)
    validation = validate_task_collection_contract(
        index_raw, items, source=normalized_index, actual_index_path=normalized_index,
        project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots, validate_file_state=False,
        _source_plan_raw=plan_raw,
    )
    framed = bytearray(b"WORK-TASK-COLLECTION-CREATE-V2\n")
    for path, content in [(normalized_index, index_raw)] + [
        (f"{normalized_index.rsplit('/', 1)[0]}/tasks/{task_id}.json", items[task_id])
        for task_id in sorted(items)
    ]:
        encoded = path.encode("utf-8")
        framed.extend(str(len(encoded)).encode("ascii") + b":" + encoded)
        framed.extend(str(len(content)).encode("ascii") + b":" + content)
    return {
        "index": index, "index_raw": index_raw, "items": items,
        "validation": validation, "approval_bytes": bytes(framed),
        "normalized_index": normalized_index,
    }


def _task_collection_create_inputs(
    raw: bytes, *, source: str, raw_plan_path: str, raw_task_path: str,
    raw_execution_dir: str, project_root: Path, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    bundle = prepare_task_collection_create(
        raw, source=source, raw_plan_path=raw_plan_path,
        raw_task_path=raw_task_path, project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
    )
    index = bundle["index"]
    artifacts = index["artifacts"]
    normalized_execution, execution_path = resolve_project_relative_path(project_root, raw_execution_dir, field="execution_dir")
    if normalized_execution != artifacts["execution"]:
        raise WorkError(ExitCode.CONTRACT, "task_create_path_mismatch", "The explicit execution path must match the TASK index.")
    initial = build_initial_execution_index(bundle["validation"]["logical_contract"], bundle["validation"])
    execution_raw = render_execution_index(initial)
    validate_execution_index(execution_raw, source="generated v2 execution index", expected=initial)
    _, index_path = resolve_project_relative_path(project_root, bundle["normalized_index"], field="task_index")
    return {**bundle, "initial_execution": initial, "execution_raw": execution_raw,
            "index_path": index_path, "execution_path": execution_path,
            "normalized_execution": normalized_execution}


def _write_v2_targets(inputs: dict[str, object]) -> bool:
    index_path = inputs["index_path"]
    execution_path = inputs["execution_path"]
    item_directory = index_path.parent / "tasks"
    expected_items = {f"{task_id}.json": raw for task_id, raw in inputs["items"].items()}
    if item_directory.exists():
        if item_directory.is_symlink() or not item_directory.is_dir():
            raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "The TASK item target is not a safe directory.")
        observed = {path.name for path in item_directory.iterdir()}
        if observed - set(expected_items):
            raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "The TASK item directory contains unknown content.")
    for name, raw in expected_items.items():
        path = item_directory / name
        if path.exists():
            if not path.is_file() or read_raw(path) != raw:
                raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "An existing TASK item conflicts with approved bytes.", {"path": str(path)})
    if index_path.exists():
        if not index_path.is_file() or read_raw(index_path) != inputs["index_raw"]:
            raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "The formal TASK index conflicts with approved bytes.")
    if execution_path.exists() and (execution_path.is_symlink() or not execution_path.is_dir()):
        raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "The execution target is not a directory.")
    execution_index = execution_path / "index.json"
    if execution_path.exists():
        entries = list(execution_path.iterdir())
        if any(path != execution_index for path in entries):
            raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "The execution directory contains unknown content.")
    if execution_index.exists():
        if not execution_index.is_file() or read_raw(execution_index) != inputs["execution_raw"]:
            raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "The execution index conflicts with approved bytes.")

    changed = False
    index_path.parent.mkdir(parents=True, exist_ok=True)
    item_directory.mkdir(exist_ok=True)
    for name, raw in expected_items.items():
        path = item_directory / name
        if not path.exists():
            _write_exclusive(path, raw, code="task_item_already_exists", label="TASK item")
            changed = True
    if not index_path.exists():
        _write_exclusive(index_path, inputs["index_raw"], code="task_index_already_exists", label="TASK index")
        changed = True
    execution_path.mkdir(parents=True, exist_ok=True)
    if not execution_index.exists():
        _write_exclusive(execution_index, inputs["execution_raw"], code="execution_index_already_exists", label="execution index")
        changed = True
    return changed


def _validate_v2_create_targets(inputs: dict[str, object]) -> None:
    index_path = inputs["index_path"]
    collection_directory = index_path.parent
    execution_path = inputs["execution_path"]
    unexpected: list[str] = []
    if collection_directory.exists():
        for path in collection_directory.iterdir():
            if path.name != "drafts" or path.is_symlink() or not path.is_dir():
                unexpected.append(path.name)
    if unexpected or index_path.exists() or collection_directory.joinpath("tasks").exists() or execution_path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "task_create_target_exists",
            "V2 TASK create requires formal collection and execution targets to be absent; only an existing drafts directory is allowed.",
            {
                "task_index_exists": index_path.exists(),
                "task_items_directory_exists": collection_directory.joinpath("tasks").exists(),
                "execution_exists": execution_path.exists(),
                "unexpected_collection_entries": sorted(unexpected),
            },
        )


def _validate_v2_recovery_root(inputs: dict[str, object]) -> None:
    collection_directory = inputs["index_path"].parent
    allowed = {"drafts", "index.json", "tasks"}
    unexpected = sorted(path.name for path in collection_directory.iterdir() if path.name not in allowed)
    drafts = collection_directory / "drafts"
    if drafts.exists() and (drafts.is_symlink() or not drafts.is_dir()):
        unexpected.append("drafts")
    if unexpected:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "V2 recovery found unknown or unsafe content in the collection directory.",
            {"unexpected_collection_entries": sorted(set(unexpected))},
        )


def _task_create_inputs(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    raw_task_path: str,
    raw_execution_dir: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[
    dict[str, Any],
    dict[str, object],
    bytes,
    dict[str, Any],
    bytes,
    str,
    Path,
    str,
    Path,
]:
    contract = parse_json_contract(raw, source=source)
    validation, rendered_task = prepare_task_json_contract(
        raw,
        source=source,
        actual_task_path=raw_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    artifacts = contract["artifacts"]
    normalized_plan, _ = resolve_project_relative_path(
        project_root, raw_plan_path, field="plan_path"
    )
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    if (
        normalized_plan != artifacts["plan"]
        or normalized_task != artifacts["task"]
        or normalized_execution != artifacts["execution"]
    ):
        raise WorkError(
            ExitCode.CONTRACT,
            "task_create_path_mismatch",
            "The explicit create paths must match the TASK artifact paths.",
        )
    initial_index = build_initial_execution_index(contract, validation)
    rendered_index = render_execution_index(initial_index)
    validate_execution_index(
        rendered_index,
        source="generated execution index",
        expected=initial_index,
    )
    return (
        contract,
        validation,
        rendered_task,
        initial_index,
        rendered_index,
        normalized_task,
        task_path,
        normalized_execution,
        execution_path,
    )


def _write_exclusive(path: Path, content: bytes, *, code: str, label: str) -> None:
    try:
        with path.open("xb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            code,
            f"The {label} target already exists.",
            {"path": str(path)},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            f"{code}_write_failed",
            f"The {label} could not be created.",
            {"path": str(path)},
        ) from error


def _validate_created_pair(
    *,
    project_root: Path,
    user_config_root: str,
    normalized_task: str,
    task_validation: dict[str, object],
    index_path: Path,
    initial_index: dict[str, Any],
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[dict[str, object], dict[str, object]]:
    stored_task = validate_task_file(
        project_root,
        user_config_root,
        normalized_task,
        skill_roots=skill_roots,
    )
    if stored_task != task_validation:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "task_post_write_mismatch",
            "The stored TASK does not match the validated canonical TASK.",
        )
    stored_index = validate_execution_index(
        read_raw(index_path),
        source=str(index_path),
        expected=initial_index,
    )
    return stored_task, stored_index


def create_task_artifacts(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    raw_task_path: str,
    raw_execution_dir: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    if raw_task_path.endswith("/index.json"):
        inputs = _task_collection_create_inputs(
            raw, source=source, raw_plan_path=raw_plan_path,
            raw_task_path=raw_task_path, raw_execution_dir=raw_execution_dir,
            project_root=project_root, user_config_root=user_config_root,
            skill_roots=skill_roots,
        )
        _validate_v2_create_targets(inputs)
        _write_v2_targets(inputs)
        stored = load_task_collection(project_root, user_config_root, raw_task_path, skill_roots=skill_roots)
        stored_index = validate_execution_index(read_raw(inputs["execution_path"] / "index.json"), source="created execution index", expected=inputs["initial_execution"])
        return {"schema": "work-task-create/v2", "requirement_id": stored["requirement_id"], "spec_id": stored["spec_id"],
                "task_path": inputs["normalized_index"], "execution_dir": inputs["normalized_execution"],
                "task_collection_sha256": stored["task_collection_sha256"], "task_index_sha256": stored["task_index_sha256"],
                "task_item_sha256": stored["task_item_sha256"], "index_sha256": stored_index["index_sha256"], "status": "created"}
    (
        _,
        task_validation,
        rendered_task,
        initial_index,
        rendered_index,
        normalized_task,
        task_path,
        normalized_execution,
        execution_path,
    ) = _task_create_inputs(
        raw,
        source=source,
        raw_plan_path=raw_plan_path,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if task_path.exists() or execution_path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "task_create_target_exists",
            "TASK create requires both the TASK and execution directory to be absent.",
            {
                "task_exists": task_path.exists(),
                "execution_exists": execution_path.exists(),
            },
        )
    try:
        task_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "task_create_parent_failed",
            "A TASK create parent directory could not be created.",
        ) from error
    _write_exclusive(
        task_path,
        rendered_task,
        code="task_already_exists",
        label="TASK",
    )
    try:
        execution_path.mkdir()
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execution_directory_already_exists",
            "The execution directory appeared after the TASK was created.",
            {"path": normalized_execution},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execution_directory_create_failed",
            "The execution directory could not be created after the TASK was created.",
            {"path": normalized_execution},
        ) from error
    index_path = execution_path / "index.json"
    _write_exclusive(
        index_path,
        rendered_index,
        code="execution_index_already_exists",
        label="execution index",
    )
    stored_task, stored_index = _validate_created_pair(
        project_root=project_root,
        user_config_root=user_config_root,
        normalized_task=normalized_task,
        task_validation=task_validation,
        index_path=index_path,
        initial_index=initial_index,
        skill_roots=skill_roots,
    )
    return {
        "schema": "work-task-create/v1",
        "requirement_id": stored_task["requirement_id"],
        "spec_id": stored_task["spec_id"],
        "task_path": normalized_task,
        "execution_dir": normalized_execution,
        "task_sha256": stored_task["task_sha256"],
        "index_sha256": stored_index["index_sha256"],
        "status": "created",
    }


def recover_task_create(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    raw_task_path: str,
    raw_execution_dir: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    if raw_task_path.endswith("/index.json"):
        inputs = _task_collection_create_inputs(
            raw, source=source, raw_plan_path=raw_plan_path,
            raw_task_path=raw_task_path, raw_execution_dir=raw_execution_dir,
            project_root=project_root, user_config_root=user_config_root,
            skill_roots=skill_roots,
        )
        if not inputs["index_path"].parent.exists():
            raise WorkError(ExitCode.WORKFLOW_STATE, "unrecoverable_task_create_state", "V2 recovery requires preserved create storage.")
        _validate_v2_recovery_root(inputs)
        changed = _write_v2_targets(inputs)
        stored = load_task_collection(project_root, user_config_root, raw_task_path, skill_roots=skill_roots)
        stored_index = validate_execution_index(read_raw(inputs["execution_path"] / "index.json"), source="recovered execution index", expected=inputs["initial_execution"])
        return {"schema": "work-task-create-recovery/v2", "requirement_id": stored["requirement_id"], "spec_id": stored["spec_id"],
                "task_path": inputs["normalized_index"], "execution_dir": inputs["normalized_execution"],
                "task_collection_sha256": stored["task_collection_sha256"], "task_index_sha256": stored["task_index_sha256"],
                "task_item_sha256": stored["task_item_sha256"], "index_sha256": stored_index["index_sha256"],
                "recovered": changed, "status": "recovered" if changed else "already_completed"}
    (
        _,
        task_validation,
        rendered_task,
        initial_index,
        rendered_index,
        normalized_task,
        task_path,
        normalized_execution,
        execution_path,
    ) = _task_create_inputs(
        raw,
        source=source,
        raw_plan_path=raw_plan_path,
        raw_task_path=raw_task_path,
        raw_execution_dir=raw_execution_dir,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if not task_path.is_file() or read_raw(task_path) != rendered_task:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "Recovery requires the same canonical TASK to already exist.",
            {"task_path": normalized_task},
        )
    if execution_path.exists() and not execution_path.is_dir():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "The execution target exists but is not a directory.",
            {"execution_dir": normalized_execution},
        )
    if not execution_path.exists():
        try:
            execution_path.parent.mkdir(parents=True, exist_ok=True)
            execution_path.mkdir()
        except OSError as error:
            raise WorkError(
                ExitCode.IO_FAILURE,
                "execution_directory_create_failed",
                "The missing execution directory could not be created during recovery.",
                {"execution_dir": normalized_execution},
            ) from error
    entries = list(execution_path.iterdir())
    index_path = execution_path / "index.json"
    if not entries:
        _write_exclusive(
            index_path,
            rendered_index,
            code="execution_index_already_exists",
            label="execution index",
        )
        recovered = True
        status = "recovered"
    elif len(entries) == 1 and entries[0] == index_path and index_path.is_file():
        if read_raw(index_path) != rendered_index:
            raise WorkError(
                ExitCode.WORKFLOW_STATE,
                "unrecoverable_task_create_state",
                "The existing execution index is not the expected initial index.",
                {"execution_dir": normalized_execution},
            )
        recovered = False
        status = "already_completed"
    else:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "unrecoverable_task_create_state",
            "The execution directory contains unknown or non-initial content.",
            {"execution_dir": normalized_execution},
        )
    stored_task, stored_index = _validate_created_pair(
        project_root=project_root,
        user_config_root=user_config_root,
        normalized_task=normalized_task,
        task_validation=task_validation,
        index_path=index_path,
        initial_index=initial_index,
        skill_roots=skill_roots,
    )
    return {
        "schema": "work-task-create-recovery/v1",
        "requirement_id": stored_task["requirement_id"],
        "spec_id": stored_task["spec_id"],
        "task_path": normalized_task,
        "execution_dir": normalized_execution,
        "task_sha256": stored_task["task_sha256"],
        "index_sha256": stored_index["index_sha256"],
        "recovered": recovered,
        "status": status,
    }
