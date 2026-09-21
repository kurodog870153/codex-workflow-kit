from __future__ import annotations

import argparse
from pathlib import Path
from typing import Protocol

from .draft_assembly import assemble_task_drafts, create_task_from_drafts
from .draft_list import update_task_planning_list
from .draft_prepare import initialize_task_planning_request, prepare_task_planning_request
from .draft_request import save_task_draft_request
from .draft_source import check_task_draft_sources
from .draft_source_update import update_task_draft_sources
from .draft_status import task_draft_status
from .creation import create_task_artifacts, recover_task_create
from .io import load_task_collection
from ...services.task.draft.storage import (
    read_task_draft,
    read_task_planning_index,
    recover_task_planning,
    save_task_planning,
)
from ...models.common.validation import ContractValuePolicy
from ...models.common.errors import ExitCode, WorkError
from ...services.task.document import parse_json_contract
from ...services.task.storage import read_raw
from ...services.specification.storage import storage_path
from ...services.skill_catalog import parse_skill_root


strict_keys = ContractValuePolicy.strict_keys


class TaskRequestInput(Protocol):
    raw: bytes
    source: str


def _require_collection_write_path(path: object) -> None:
    if not isinstance(path, str) or not path.endswith("/index.json"):
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "task_collection_required",
            "TASK writes require a collection index.json artifact.",
        )


def _require_collection_plan(project_root: Path, plan_path: object) -> None:
    if not isinstance(plan_path, str):
        _require_collection_write_path(None)
    plan = parse_json_contract(read_raw(storage_path(project_root, plan_path)), source=plan_path)
    _require_collection_write_path(plan.get("artifacts", {}).get("task") if isinstance(plan, dict) else None)


def _draft_selection_arguments(arguments: argparse.Namespace) -> dict[str, object]:
    if arguments.general_only or arguments.instruction_path is not None:
        return {"selected_paths": arguments.instruction_path or [], "reference_names": arguments.reference or []}
    if arguments.reference is not None:
        raise WorkError(ExitCode.CLI_USAGE, "draft_selection_incomplete", "--reference requires --general-only or --instruction-path.")
    return {"selected_paths": None, "reference_names": None}


def _specification_summary(result: dict[str, object]) -> dict[str, object]:
    source = result.get("preview", result)
    summary = {
        "schema": "work-specification-summary/v1",
        "status": source["status"],
        "record_id": source["record_id"],
        "approved_sha256": source["approved_sha256"],
        "affected_task_ids": source["affected_task_ids"],
        "changed_fields": source["changed_fields"],
        "file_readiness": source["file_readiness"],
    }
    if "preview" in result:
        summary["output_file"] = result.get("output_file")
        summary["transport"] = result["transport"]
        summary["next_step"] = result["next_step"]
    elif "next_step" in source:
        summary["next_step"] = source["next_step"]
    if "verification_request" in source:
        summary["verification_request"] = source["verification_request"]
    return summary


def execute_task_command(
    arguments: argparse.Namespace,
    project_root: Path,
    request: TaskRequestInput | None,
    *,
    prepare_specification,
    preview_specification_migration,
    preview_specification_reconciliation,
    publish_specification_migration,
    publish_specification_reconciliation,
    update_specification,
    verify_specification,
    prepare_task_repair,
    repair_task,
    diagnose_task_collection,
    draft_operations,
) -> dict[str, object]:
    if arguments.task_command == "migration-preview":
        return preview_specification_migration(
            request.raw, project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.task_command in {"migration-apply", "migration-recover"}:
        return publish_specification_migration(
            request.raw, project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            operation="apply" if arguments.task_command == "migration-apply" else "recover",
            approved_sha256=arguments.approved_sha256,
        )
    if arguments.task_command == "reconciliation-preview":
        return preview_specification_reconciliation(
            request.raw, project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.task_command == "reconciliation-apply":
        return publish_specification_reconciliation(
            request.raw, approved_sha256=arguments.approved_sha256,
            project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.task_command in {"draft-init-request", "draft-list-prepare"}:
        options = dict(plan_path=arguments.plan_path, user_config_root=arguments.user_config_root,
                       skill_roots=[parse_skill_root(root) for root in arguments.skill_root])
        payload = parse_json_contract(request.raw, source=request.source)
        if arguments.task_command == "draft-init-request":
            return initialize_task_planning_request(project_root, arguments.requirement_id, payload,
                prepare_only=arguments.prepare_only, operations=draft_operations, **options)
        if arguments.expected_revision < 1:
            raise WorkError(ExitCode.CLI_USAGE, "invalid_expected_revision", "List preparation requires an existing revision.")
        return prepare_task_planning_request(project_root, arguments.requirement_id, payload,
            expected_revision=arguments.expected_revision, operations=draft_operations, **options)
    if arguments.task_command in {"spec-prepare", "repair-prepare"}:
        if arguments.task_command == "spec-prepare":
            _require_collection_plan(project_root, parse_json_contract(request.raw, source=request.source).get("plan_path"))
        elif arguments.task_command == "repair-prepare":
            _require_collection_write_path(parse_json_contract(request.raw, source=request.source).get("artifacts", {}).get("task"))
        prepare = {"spec-prepare": prepare_specification,
                   "repair-prepare": prepare_task_repair}[arguments.task_command]
        result = prepare(
            request.raw, project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            output_file=arguments.output_file,
        )
        return _specification_summary(result) if getattr(arguments, "summary", False) else result
    if arguments.task_command == "spec-verify":
        report = verify_specification(
            request.raw, project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
        if not report["verified"]:
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY, "specification_verification_failed",
                "Specification verification found incomplete or changed evidence; review the report.", report,
            )
        return report
    if arguments.task_command in {"repair-validate", "repair", "repair-recover"}:
        _require_collection_write_path(parse_json_contract(request.raw, source=request.source).get("artifacts", {}).get("task"))
        return repair_task(
            request.raw, project_root=project_root, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            operation={"repair-validate": "validate", "repair": "apply", "repair-recover": "recover"}[arguments.task_command],
            approved_sha256=getattr(arguments, "approved_sha256", None),
        )
    if arguments.task_command in {"spec-validate", "spec-update", "spec-recover"}:
        payload = parse_json_contract(request.raw, source=request.source)
        _require_collection_write_path(payload.get("plan", {}).get("artifacts", {}).get("task"))
        result = update_specification(
            request.raw, project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            operation={"spec-validate": "validate", "spec-update": "apply", "spec-recover": "recover"}[arguments.task_command],
            approved_sha256=getattr(arguments, "approved_sha256", None),
        )
        return _specification_summary(result) if getattr(arguments, "summary", False) else result
    if arguments.task_command in {"draft-list-update", "draft-list-recover"}:
        request = strict_keys(
            parse_json_contract(request.raw, source=request.source),
            location="draft_list_update", required={"index", "reason"},
        )
        return update_task_planning_list(
            project_root, request["index"], expected_revision=arguments.expected_revision,
            reason=request["reason"], recover=arguments.task_command == "draft-list-recover",
        )
    if arguments.task_command == "draft-status":
        return task_draft_status(project_root, arguments.requirement_id, task_id=arguments.task_id)
    if arguments.task_command == "draft-read":
        if arguments.task_id is not None:
            return read_task_draft(project_root, arguments.requirement_id, arguments.task_id)
        return read_task_planning_index(project_root, arguments.requirement_id)
    if arguments.task_command in {"draft-init", "draft-save", "draft-recover"}:
        payload = parse_json_contract(request.raw, source=request.source)
        if arguments.task_command == "draft-init":
            return save_task_planning(project_root, payload, expected_revision=0)
        if arguments.task_command == "draft-recover" and arguments.expected_revision == 0:
            return recover_task_planning(project_root, payload, expected_revision=0)
        request = strict_keys(payload, location="draft_save", required={"index", "draft"})
        if not isinstance(request["draft"], dict):
            raise WorkError(ExitCode.CONTRACT, "expected_object", "draft must be a JSON object.")
        operation = recover_task_planning if arguments.task_command == "draft-recover" else save_task_planning
        return operation(
            project_root, request["index"],
            expected_revision=arguments.expected_revision, draft=request["draft"],
        )
    skill_roots = [parse_skill_root(root) for root in arguments.skill_root]
    if arguments.task_command in {"draft-save-request", "draft-recover-request"}:
        return save_task_draft_request(
            project_root, arguments.requirement_id, arguments.task_id,
            parse_json_contract(request.raw, source=request.source),
            expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
            user_config_root=arguments.user_config_root, skill_roots=skill_roots,
            **_draft_selection_arguments(arguments),
            recover=arguments.task_command == "draft-recover-request",
            operations=draft_operations,
        )
    if arguments.task_command in {"draft-source-update", "draft-source-recover"}:
        return update_task_draft_sources(
            project_root, arguments.requirement_id,
            parse_json_contract(request.raw, source=request.source),
            expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
            user_config_root=arguments.user_config_root, skill_roots=skill_roots,
            recover=arguments.task_command == "draft-source-recover",
            operations=draft_operations,
        )
    if arguments.task_command in {"draft-assemble", "draft-create"}:
        if arguments.task_command == "draft-create":
            _require_collection_plan(project_root, arguments.plan_path)
        metadata = parse_json_contract(request.raw, source=request.source)
        options = dict(expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
                       user_config_root=arguments.user_config_root, skill_roots=skill_roots)
        if arguments.task_command == "draft-create":
            return create_task_from_drafts(project_root, arguments.requirement_id, metadata,
                                           approved_sha256=arguments.approved_sha256,
                                           operations=draft_operations, **options)
        return assemble_task_drafts(project_root, arguments.requirement_id, metadata,
                                    operations=draft_operations, **options)
    if arguments.task_command == "draft-check":
        return check_task_draft_sources(
            project_root, arguments.requirement_id, arguments.task_id,
            expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
            user_config_root=arguments.user_config_root, skill_roots=skill_roots,
            **_draft_selection_arguments(arguments),
            operations=draft_operations,
        )
    if arguments.task_command in {"create", "recover-create"}:
        _require_collection_write_path(arguments.task_path)
        operation = (
            create_task_artifacts
            if arguments.task_command == "create"
            else recover_task_create
        )
        return operation(
            request.raw,
            source=request.source,
            raw_plan_path=arguments.plan_path,
            raw_task_path=arguments.task_path,
            raw_execution_dir=arguments.execution_dir,
            project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=skill_roots,
        )
    if arguments.task_command == "diagnose":
        report = diagnose_task_collection(
            project_root,
            arguments.user_config_root,
            arguments.path,
            skill_roots=skill_roots,
        )
        if not report["normal_use_allowed"]:
            raise WorkError(
                ExitCode.CONTRACT, "task_diagnostics_failed",
                "TASK validation is blocked; review the diagnostic report.", report,
            )
        return report
    if arguments.input_file:
        if not arguments.task_path:
            raise WorkError(
                ExitCode.CLI_USAGE,
                "task_path_required",
                "--task-path is required with --input-file.",
            )
        return load_task_collection(
            project_root,
            arguments.user_config_root,
            arguments.task_path,
            skill_roots=skill_roots,
            raw=request.raw,
        )
    if arguments.task_path:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "unexpected_task_path",
            "--task-path is only valid with --input-file.",
        )
    return load_task_collection(
        project_root,
        arguments.user_config_root,
        arguments.path,
        skill_roots=skill_roots,
    )
