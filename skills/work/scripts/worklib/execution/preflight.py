from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..contracts.execution_index import validate_execution_index
from ..foundation.fingerprint import read_raw
from ..instructions.selection import build_instruction_selection
from ..foundation.markdown import parse_markdown_json_contract
from ..foundation.paths import portable_path_identity, resolve_project_relative_path
from ..skills.catalog import SkillRoot
from ..skills.selection import selection_sha256
from ..contracts.task import validate_task_contract


BASE_EXECUTE_REFERENCES = ["execute.general.execution-records"]
RECOVERY_REFERENCE = "execute.general.execution-recovery"
ELIGIBLE_NEW_ATTEMPT_STATUSES = {"pending", "pending_retry"}
CONFIRMED_INPUT_PATTERN = re.compile(r"^TASK-\d{3}/INPUT-\d{3}$")
ATTEMPT_START_TRANSACTION_PATTERN = re.compile(
    r"^\.work-attempt-start-TASK-\d{3}-ATTEMPT-\d{3}-(?:lock|started)\.tmp$"
)


def _require_file(path: Path, *, code: str, message: str) -> None:
    if not path.is_file():
        raise WorkError(
            ExitCode.IO_FAILURE,
            code,
            message,
            {"path": str(path)},
        )


def _index_contract(raw: bytes, *, source: str) -> dict[str, Any]:
    validate_execution_index(raw, source=source)
    _, contract = parse_markdown_json_contract(raw, source=source)
    return contract


def _file_output_path(item: dict[str, Any]) -> str:
    return item["destination"] if item["action"] == "move" else item["path"]


def _validate_inputs(
    *,
    project_root: Path,
    task: dict[str, Any],
    tasks: dict[str, dict[str, Any]],
    confirmed_inputs: list[str],
) -> list[dict[str, str]]:
    if len(confirmed_inputs) != len(set(confirmed_inputs)):
        raise WorkError(
            ExitCode.CONTRACT,
            "execute_preflight_duplicate_confirmed_input",
            "Confirmed input IDs must be unique.",
        )
    if any(not CONFIRMED_INPUT_PATTERN.fullmatch(value) for value in confirmed_inputs):
        raise WorkError(
            ExitCode.CONTRACT,
            "execute_preflight_invalid_confirmed_input",
            "A confirmed input must use TASK-nnn/INPUT-nnn format.",
            {"confirmed_inputs": confirmed_inputs},
        )

    inputs = task.get("inputs", [])
    manual_ids = {
        f"{task['id']}/{item['id']}"
        for item in inputs
        if item["kind"] in {"user_provided", "external"}
    }
    unknown = sorted(set(confirmed_inputs) - manual_ids)
    if unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "execute_preflight_unknown_confirmed_input",
            "A confirmed input is not a manual input of the requested TASK.",
            {"confirmed_inputs": unknown},
        )
    missing = sorted(manual_ids - set(confirmed_inputs))
    if missing:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execute_preflight_input_confirmation_required",
            "A user-provided or external input requires explicit confirmation.",
            {"required_confirmations": missing},
        )

    results: list[dict[str, str]] = []
    for item in inputs:
        result = {"id": item["id"], "kind": item["kind"], "status": "ready"}
        if item["kind"] == "project_state":
            normalized, path = resolve_project_relative_path(
                project_root,
                item["source"],
                field=f"{task['id']}.{item['id']}.source",
            )
            if not path.exists():
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "execute_preflight_project_state_missing",
                    "A project_state input does not exist.",
                    {"input_id": item["id"], "path": normalized},
                )
            result["resolved_source"] = normalized
        elif item["kind"] == "task_output":
            source_task_id, source_file_id = item["source"].split("/", 1)
            source_task = tasks[source_task_id]
            source_file = next(
                file_item
                for file_item in source_task.get("files", [])
                if file_item["id"] == source_file_id
            )
            output_path = _file_output_path(source_file)
            normalized, path = resolve_project_relative_path(
                project_root,
                output_path,
                field=f"{task['id']}.{item['id']}.source",
            )
            if not path.exists():
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "execute_preflight_task_output_missing",
                    "A completed dependency TASK output does not exist.",
                    {"input_id": item["id"], "path": normalized},
                )
            result["resolved_source"] = normalized
        results.append(result)
    return results


def _validate_file_lifecycle(
    *, project_root: Path, task: dict[str, Any]
) -> list[dict[str, str]]:
    files = task.get("files", [])
    paths = {
        item[field]
        for item in files
        for field in ("path", "source", "destination")
        if field in item
    }
    path_identities: dict[str, str] = {}
    identity_paths: dict[str, list[Path]] = {}
    for raw_path in paths:
        _, path = resolve_project_relative_path(
            project_root, raw_path, field=f"{task['id']}.files"
        )
        identity = portable_path_identity(path)
        path_identities[raw_path] = identity
        identity_paths.setdefault(identity, []).append(path)
    existence = {
        identity: any(path.exists() for path in resolved_paths)
        for identity, resolved_paths in identity_paths.items()
    }

    results: list[dict[str, str]] = []
    for item in files:
        action = item["action"]
        result = {"id": item["id"], "action": action, "status": "ready"}
        if action == "create":
            path = item["path"]
            identity = path_identities[path]
            if existence[identity]:
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "execute_preflight_create_target_exists",
                    "A create target already exists.",
                    {"file_id": item["id"], "path": path},
                )
            existence[identity] = True
            result["path"] = path
        elif action == "modify":
            path = item["path"]
            identity = path_identities[path]
            if not existence[identity]:
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "execute_preflight_modify_target_missing",
                    "A modify target does not exist.",
                    {"file_id": item["id"], "path": path},
                )
            result["path"] = path
        else:
            source = item["source"]
            destination = item["destination"]
            source_identity = path_identities[source]
            destination_identity = path_identities[destination]
            if not existence[source_identity]:
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "execute_preflight_move_source_missing",
                    "A move source does not exist.",
                    {"file_id": item["id"], "path": source},
                )
            if existence[destination_identity]:
                raise WorkError(
                    ExitCode.WORKFLOW_STATE,
                    "execute_preflight_move_destination_exists",
                    "A move destination already exists.",
                    {"file_id": item["id"], "path": destination},
                )
            existence[source_identity] = False
            existence[destination_identity] = True
            result["source"] = source
            result["destination"] = destination
        results.append(result)
    return results


def _task_contract(
    raw: bytes,
    *,
    source: str,
    task_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[dict[str, Any], dict[str, object]]:
    validation = validate_task_contract(
        raw,
        source=source,
        actual_task_path=task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=False,
        skill_roots=skill_roots,
    )
    _, contract = parse_markdown_json_contract(raw, source=source)
    return contract, validation


def _require_index_identity(
    index: dict[str, Any],
    task: dict[str, Any],
    task_validation: dict[str, object],
) -> dict[str, dict[str, Any]]:
    expected_identity = {
        "requirement_id": task["requirement_id"],
        "task_spec_id": task["spec_id"],
        "task_sha256": task_validation["task_sha256"],
        "task_instructions_sha256": task_validation["instructions_sha256"],
        "hierarchy_selection_sha256": task_validation[
            "hierarchy_selection_sha256"
        ],
    }
    observed_identity = {key: index[key] for key in expected_identity}
    if observed_identity != expected_identity:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            {"expected": expected_identity, "observed": observed_identity},
        )

    task_instructions = task_validation["task_instructions_sha256"]
    assert isinstance(task_instructions, dict)
    expected_ids = [item["id"] for item in task["tasks"]]
    observed_ids = [item["id"] for item in index["tasks"]]
    if observed_ids != expected_ids:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK document.",
            {"expected": expected_ids, "observed": observed_ids},
        )

    rows = {item["id"]: item for item in index["tasks"]}
    mismatches = {
        task_id: {
            "expected": task_instructions[task_id],
            "observed": rows[task_id]["instructions_sha256"],
        }
        for task_id in expected_ids
        if rows[task_id]["instructions_sha256"] != task_instructions[task_id]
    }
    if mismatches:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_task_instructions_mismatch",
            "The execution index per-TASK instruction fingerprints are stale.",
            {"tasks": mismatches},
        )
    return rows


def execute_preflight(
    *,
    project_root: Path,
    user_config_root: str,
    raw_task_path: str,
    raw_execution_dir: str,
    task_id: str,
    confirmed_inputs: list[str] | None = None,
    skill_roots: list[SkillRoot] | None = None,
    _allowed_lock: dict[str, Any] | None = None,
    _allow_attempt_start_transaction: bool = False,
    _eligible_statuses: set[str] | None = None,
    _rule_status: str | None = None,
) -> dict[str, object]:
    normalized_task, task_path = resolve_project_relative_path(
        project_root, raw_task_path, field="task_path"
    )
    normalized_execution, execution_path = resolve_project_relative_path(
        project_root, raw_execution_dir, field="execution_dir"
    )
    _require_file(
        task_path,
        code="execute_preflight_task_missing",
        message="The formal TASK document does not exist.",
    )
    if not execution_path.is_dir():
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execute_preflight_execution_directory_missing",
            "The execution directory does not exist.",
            {"path": str(execution_path)},
        )
    transaction_files = sorted(
        path.name
        for path in execution_path.iterdir()
        if path.is_file() and ATTEMPT_START_TRANSACTION_PATTERN.fullmatch(path.name)
    )
    if transaction_files and not _allow_attempt_start_transaction:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "execute_preflight_attempt_start_transaction_present",
            "An incomplete Attempt-start transaction requires recovery.",
            {"files": transaction_files},
        )

    task_raw = read_raw(task_path)
    task_contract, task_validation = _task_contract(
        task_raw,
        source=str(task_path),
        task_path=normalized_task,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    artifacts = task_contract["artifacts"]
    if (
        artifacts["task"] != normalized_task
        or artifacts["execution"] != normalized_execution
    ):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match the formal artifacts.",
        )

    normalized_index, index_path = resolve_project_relative_path(
        project_root,
        f"{normalized_execution}/index.md",
        field="execution_index",
    )
    _require_file(
        index_path,
        code="execute_preflight_index_missing",
        message="The canonical execution index does not exist.",
    )
    index_raw = read_raw(index_path)
    index_validation = validate_execution_index(index_raw, source=str(index_path))
    _, index_contract = parse_markdown_json_contract(index_raw, source=str(index_path))
    index_rows = _require_index_identity(
        index_contract, task_contract, task_validation
    )

    tasks = {item["id"]: item for item in task_contract["tasks"]}
    if task_id not in tasks:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execute_preflight_task_not_found",
            "The requested TASK ID does not exist in the formal TASK document.",
            {"task_id": task_id},
        )
    if "lock" in index_contract and index_contract["lock"] != _allowed_lock:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "execute_preflight_lock_present",
            "The execution index already contains a lock.",
            {"lock": index_contract["lock"]},
        )

    task_row = index_rows[task_id]
    status = task_row["status"]
    eligible_statuses = _eligible_statuses or ELIGIBLE_NEW_ATTEMPT_STATUSES
    if status not in eligible_statuses:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execute_preflight_task_not_eligible",
            "The requested TASK status is not eligible for a new Attempt.",
            {"task_id": task_id, "status": status},
        )

    task = tasks[task_id]
    _, plan_path = resolve_project_relative_path(
        project_root, artifacts["plan"], field="plan_path"
    )
    _, plan_contract = parse_markdown_json_contract(
        read_raw(plan_path), source=str(plan_path)
    )
    plan_skill_selection = plan_contract["skill_selection"]
    assert isinstance(plan_skill_selection, dict)
    target_skill_id = task["skill_id"]
    selected_skills = [
        skill
        for skill in plan_skill_selection["skills"]
        if isinstance(skill, dict) and skill["id"] == target_skill_id
    ]
    decision = "external_skills" if target_skill_id is not None else "base_only"
    execute_skill_selection = {
        "schema": "work-skill-selection/v1",
        "decision": decision,
        "skills": selected_skills,
        "selection_sha256": selection_sha256(decision, selected_skills),
    }
    dependencies = task.get("dependencies", [])
    incomplete_dependencies = [
        dependency
        for dependency in dependencies
        if index_rows[dependency]["status"] != "completed"
    ]
    if incomplete_dependencies:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "execute_preflight_dependency_incomplete",
            "A direct TASK dependency is not completed.",
            {
                "task_id": task_id,
                "dependencies": [
                    {
                        "task_id": dependency,
                        "status": index_rows[dependency]["status"],
                    }
                    for dependency in incomplete_dependencies
                ],
            },
        )

    confirmed = list(confirmed_inputs or [])
    input_readiness = _validate_inputs(
        project_root=project_root,
        task=task,
        tasks=tasks,
        confirmed_inputs=confirmed,
    )
    file_readiness = _validate_file_lifecycle(
        project_root=project_root,
        task=task,
    )

    task_selection = task["instruction_selection"]
    execute_references = list(BASE_EXECUTE_REFERENCES)
    rule_status = _rule_status or status
    if rule_status == "pending_retry":
        execute_references.append(RECOVERY_REFERENCE)
    execute_selection = build_instruction_selection(
        skill_root=Path(__file__).resolve().parents[3],
        mode="execute",
        selected_paths=task_selection["selected_paths"],
        reference_names=execute_references,
    )
    if (
        execute_selection["selected_paths"]
        != task_selection["selected_paths"]
        or execute_selection["resolved_paths"]
        != task_selection["resolved_paths"]
    ):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "execute_preflight_instruction_hierarchy_mismatch",
            "The Execute hierarchy does not match the target TASK hierarchy.",
        )

    return {
        "schema": "work-execute-preflight/v1",
        "requirement_id": task_contract["requirement_id"],
        "task_spec_id": task_contract["spec_id"],
        "task_id": task_id,
        "skill_id": target_skill_id,
        "task_path": normalized_task,
        "execution_dir": normalized_execution,
        "index_path": normalized_index,
        "task_status": status,
        "dependencies": dependencies,
        "confirmed_inputs": confirmed,
        "inputs": input_readiness,
        "files": file_readiness,
        "task_sha256": task_validation["task_sha256"],
        "hierarchy_selection_sha256": task_validation[
            "hierarchy_selection_sha256"
        ],
        "task_instructions_sha256": task_row["instructions_sha256"],
        "execute_instructions_sha256": execute_selection["instructions_sha256"],
        "execute_instruction_selection": execute_selection,
        "execute_skill_selection": execute_skill_selection,
        "index_sha256": index_validation["index_sha256"],
        "eligibility": "passed",
    }
