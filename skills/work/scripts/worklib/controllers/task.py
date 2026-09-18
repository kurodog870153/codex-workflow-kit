from __future__ import annotations

import argparse
from pathlib import Path

from ..services.specification import prepare_specification, update_specification, verify_specification
from ..services.specification_migration import preview_specification_migration, publish_specification_migration
from ..services.specification_reconciliation import (
    preview_specification_reconciliation, publish_specification_reconciliation,
)
from ..services.task_repair import prepare_task_repair, repair_task
from ..artifacts.task import create_task_artifacts, recover_task_create
from ..artifacts.task_draft import (
    read_task_draft,
    read_task_planning_index,
    recover_task_planning,
    save_task_planning,
)
from ..contracts.task_diagnostics import diagnose_task_collection
from ..services.task_collection import load_task_collection
from ..artifacts.task_draft_sources import check_task_draft_sources
from ..artifacts.task_draft_list import update_task_planning_list
from ..artifacts.task_draft_assembly import assemble_task_drafts, create_task_from_drafts
from ..artifacts.task_draft_source_update import update_task_draft_sources
from ..artifacts.task_draft_request import save_task_draft_request
from ..artifacts.task_draft_status import task_draft_status
from ..artifacts.task_draft_prepare import initialize_task_planning_request, prepare_task_planning_request
from ..contracts.validation import strict_keys
from ..models.common.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..foundation.fingerprint import read_raw
from ..foundation.spec_update import storage_path
from ..services.skill_catalog import parse_skill_root
from ..infrastructure.cli_io import FileInput
from . import SubparserRegistry


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


def _add_draft_source_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--requirement-id", required=True)
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--expected-revision", type=int, required=True)
    parser.add_argument("--plan-path", required=True)
    parser.add_argument("--user-config-root", required=True)
    parser.add_argument("--skill-root", action="append", default=[])
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--general-only", action="store_true")
    selection.add_argument("--instruction-path", action="append")
    parser.add_argument("--reference", action="append")


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


def register_task_commands(commands: SubparserRegistry) -> None:
    task_parser = commands.add_parser("task")
    task_commands = task_parser.add_subparsers(dest="task_command", required=True)

    for name in ("spec-prepare", "repair-prepare"):
        prepare = task_commands.add_parser(name, help="Prepare validated field replacements without publishing.")
        prepare.add_argument("--input-file", required=True)
        prepare.add_argument("--user-config-root", required=True)
        prepare.add_argument("--skill-root", action="append", default=[])
        prepare.add_argument("--output-file")
        if name != "repair-prepare":
            prepare.add_argument("--summary", action="store_true")

    for name in ("spec-validate", "spec-update", "spec-recover"):
        spec = task_commands.add_parser(name, help="Internal coordinated specification revision.")
        spec.add_argument("--input-file", required=True)
        spec.add_argument("--user-config-root", required=True)
        spec.add_argument("--skill-root", action="append", default=[])
        if name != "spec-validate":
            spec.add_argument("--approved-sha256", required=True)
        if name in {"spec-validate", "spec-update"}:
            spec.add_argument("--summary", action="store_true")

    for name in ("repair-validate", "repair", "repair-recover"):
        repair = task_commands.add_parser(name, help="Review or publish an explicitly decided TASK repair.")
        repair.add_argument("--input-file", required=True)
        repair.add_argument("--user-config-root", required=True)
        repair.add_argument("--skill-root", action="append", default=[])
        if name != "repair-validate":
            repair.add_argument("--approved-sha256", required=True)

    for name in ("spec-verify",):
        inspection = task_commands.add_parser(name, help="Inspect specification evidence without writes.")
        inspection.add_argument("--input-file", required=True)
        inspection.add_argument("--user-config-root", required=True)
        inspection.add_argument("--skill-root", action="append", default=[])

    migration = task_commands.add_parser("migration-preview", help="Validate an AI-produced cross-file migration without writing.")
    migration.add_argument("--input-file", required=True)
    migration.add_argument("--user-config-root", required=True)
    migration.add_argument("--skill-root", action="append", default=[])
    for name in ("migration-apply", "migration-recover"):
        publication = task_commands.add_parser(name, help="Publish or recover an approved cross-file migration.")
        publication.add_argument("--input-file", required=True)
        publication.add_argument("--user-config-root", required=True)
        publication.add_argument("--skill-root", action="append", default=[])
        publication.add_argument("--approved-sha256", required=True)
    reconciliation = task_commands.add_parser("reconciliation-preview", help="Review specification candidates derived from execution deviations.")
    reconciliation.add_argument("--input-file", required=True)
    reconciliation.add_argument("--user-config-root", required=True)
    reconciliation.add_argument("--skill-root", action="append", default=[])
    reconciliation_apply = task_commands.add_parser("reconciliation-apply", help="Publish an approved specification reconciliation.")
    reconciliation_apply.add_argument("--input-file", required=True)
    reconciliation_apply.add_argument("--user-config-root", required=True)
    reconciliation_apply.add_argument("--skill-root", action="append", default=[])
    reconciliation_apply.add_argument("--approved-sha256", required=True)

    draft_init = task_commands.add_parser("draft-init", help="Save an initial planning index from a JSON request file.")
    draft_init.add_argument("--input-file", required=True)
    for name in ("draft-init-request", "draft-list-prepare"):
        preparation = task_commands.add_parser(name)
        preparation.add_argument("--input-file", required=True)
        preparation.add_argument("--requirement-id", required=True)
        preparation.add_argument("--plan-path", required=True)
        preparation.add_argument("--user-config-root", required=True)
        preparation.add_argument("--skill-root", action="append", default=[])
        if name == "draft-list-prepare":
            preparation.add_argument("--expected-revision", type=int, required=True)
        else:
            preparation.add_argument("--prepare-only", action="store_true")
    draft_save = task_commands.add_parser("draft-save", help="Save one discussion from an index/draft JSON object.")
    draft_save.add_argument("--input-file", required=True)
    draft_save.add_argument("--expected-revision", type=int, required=True)
    draft_recover = task_commands.add_parser("draft-recover", help="Recover a fully prepared save using its original JSON request.")
    draft_recover.add_argument("--input-file", required=True)
    draft_recover.add_argument("--expected-revision", type=int, required=True)
    draft_read = task_commands.add_parser("draft-read", help="Read the committed index or one historical draft.")
    draft_read.add_argument("--requirement-id", required=True)
    draft_read.add_argument("--task-id")
    draft_status = task_commands.add_parser("draft-status", help="Inspect planning progress and the next action without writing.")
    draft_status.add_argument("--requirement-id", required=True)
    draft_status.add_argument("--task-id")
    draft_check = task_commands.add_parser("draft-check", help="Verify one TASK's saved source fingerprints without writing.")
    _add_draft_source_arguments(draft_check)
    for command_name in ("draft-save-request", "draft-recover-request"):
        draft_request = task_commands.add_parser(command_name, help="Save or recover one discussion with derived index and draft metadata.")
        _add_draft_source_arguments(draft_request)
        draft_request.add_argument("--input-file", required=True)
    for command_name in ("draft-list-update", "draft-list-recover"):
        draft_list = task_commands.add_parser(command_name)
        draft_list.add_argument("--input-file", required=True)
        draft_list.add_argument("--expected-revision", type=int, required=True)

    diagnose = task_commands.add_parser("diagnose", help="Diagnose an existing TASK without writing.")
    diagnose.add_argument("--path", required=True)
    diagnose.add_argument("--plan-path", required=True)
    diagnose.add_argument("--execution-dir", required=True)
    diagnose.add_argument("--user-config-root", required=True)
    diagnose.add_argument("--skill-root", action="append", default=[])

    task_validate = task_commands.add_parser("validate")
    for command_name in ("draft-source-update", "draft-source-recover"):
        source_update = task_commands.add_parser(command_name)
        source_update.add_argument("--input-file", required=True)
        source_update.add_argument("--requirement-id", required=True)
        source_update.add_argument("--expected-revision", type=int, required=True)
        source_update.add_argument("--plan-path", required=True)
        source_update.add_argument("--user-config-root", required=True)
        source_update.add_argument("--skill-root", action="append", default=[])
    for command_name in ("draft-assemble", "draft-create"):
        assembly = task_commands.add_parser(command_name)
        assembly.add_argument("--input-file", required=True)
        assembly.add_argument("--requirement-id", required=True)
        assembly.add_argument("--expected-revision", type=int, required=True)
        assembly.add_argument("--plan-path", required=True)
        assembly.add_argument("--user-config-root", required=True)
        assembly.add_argument("--skill-root", action="append", default=[])
        if command_name == "draft-create":
            assembly.add_argument("--approved-sha256", required=True)
    task_validate.add_argument("--user-config-root", required=True)
    task_validate.add_argument("--skill-root", action="append", default=[])
    task_source = task_validate.add_mutually_exclusive_group(required=True)
    task_source.add_argument("--path")
    task_source.add_argument("--input-file")
    task_validate.add_argument("--task-path")

    for command_name in ("create", "recover-create"):
        task_write = task_commands.add_parser(command_name)
        task_write.add_argument("--user-config-root", required=True)
        task_write.add_argument("--skill-root", action="append", default=[])
        task_write.add_argument("--input-file", required=True)
        task_write.add_argument("--plan-path", required=True)
        task_write.add_argument("--task-path", required=True)
        task_write.add_argument("--execution-dir", required=True)


def run_task(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
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
                prepare_only=arguments.prepare_only, **options)
        if arguments.expected_revision < 1:
            raise WorkError(ExitCode.CLI_USAGE, "invalid_expected_revision", "List preparation requires an existing revision.")
        return prepare_task_planning_request(project_root, arguments.requirement_id, payload,
            expected_revision=arguments.expected_revision, **options)
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
        )
    if arguments.task_command in {"draft-source-update", "draft-source-recover"}:
        return update_task_draft_sources(
            project_root, arguments.requirement_id,
            parse_json_contract(request.raw, source=request.source),
            expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
            user_config_root=arguments.user_config_root, skill_roots=skill_roots,
            recover=arguments.task_command == "draft-source-recover",
        )
    if arguments.task_command in {"draft-assemble", "draft-create"}:
        if arguments.task_command == "draft-create":
            _require_collection_plan(project_root, arguments.plan_path)
        metadata = parse_json_contract(request.raw, source=request.source)
        options = dict(expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
                       user_config_root=arguments.user_config_root, skill_roots=skill_roots)
        if arguments.task_command == "draft-create":
            return create_task_from_drafts(project_root, arguments.requirement_id, metadata,
                                           approved_sha256=arguments.approved_sha256, **options)
        return assemble_task_drafts(project_root, arguments.requirement_id, metadata, **options)
    if arguments.task_command == "draft-check":
        return check_task_draft_sources(
            project_root, arguments.requirement_id, arguments.task_id,
            expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
            user_config_root=arguments.user_config_root, skill_roots=skill_roots,
            **_draft_selection_arguments(arguments),
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
