from __future__ import annotations

import argparse
from pathlib import Path
from typing import TextIO

from ..artifacts.specification import update_specification
from ..artifacts.task import create_task_artifacts, recover_task_create
from ..artifacts.task_draft import (
    read_task_draft,
    read_task_planning_index,
    recover_task_planning,
    save_task_planning,
)
from ..contracts.task import validate_task_file, validate_task_json_contract
from ..artifacts.task_draft_sources import check_task_draft_sources
from ..artifacts.task_draft_list import update_task_planning_list
from ..artifacts.task_draft_assembly import assemble_task_drafts, create_task_from_drafts
from ..artifacts.task_draft_source_update import update_task_draft_sources
from ..contracts.validation import strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract
from ..skills.catalog import parse_skill_root
from . import SubparserRegistry


def register_task_commands(commands: SubparserRegistry) -> None:
    task_parser = commands.add_parser("task")
    task_commands = task_parser.add_subparsers(dest="task_command", required=True)

    for name in ("spec-validate", "spec-update", "spec-recover"):
        spec = task_commands.add_parser(name, help="Internal coordinated specification revision.")
        spec.add_argument("--stdin", action="store_true", required=True)
        spec.add_argument("--user-config-root", required=True)
        spec.add_argument("--skill-root", action="append", default=[])
        if name != "spec-validate":
            spec.add_argument("--approved-sha256", required=True)

    draft_init = task_commands.add_parser("draft-init", help="Save an initial planning index from stdin.")
    draft_init.add_argument("--stdin", action="store_true", required=True)
    draft_save = task_commands.add_parser("draft-save", help="Save one discussion from an index/draft JSON object.")
    draft_save.add_argument("--stdin", action="store_true", required=True)
    draft_save.add_argument("--expected-revision", type=int, required=True)
    draft_recover = task_commands.add_parser("draft-recover", help="Recover a fully prepared save using its original JSON request.")
    draft_recover.add_argument("--stdin", action="store_true", required=True)
    draft_recover.add_argument("--expected-revision", type=int, required=True)
    draft_read = task_commands.add_parser("draft-read", help="Read the committed index or one historical draft.")
    draft_read.add_argument("--requirement-id", required=True)
    draft_read.add_argument("--task-id")
    draft_check = task_commands.add_parser("draft-check", help="Verify one TASK's saved source fingerprints without writing.")
    draft_check.add_argument("--requirement-id", required=True)
    draft_check.add_argument("--task-id", required=True)
    draft_check.add_argument("--expected-revision", type=int, required=True)
    draft_check.add_argument("--plan-path", required=True)
    draft_check.add_argument("--user-config-root", required=True)
    draft_check.add_argument("--skill-root", action="append", default=[])
    selection = draft_check.add_mutually_exclusive_group(required=True)
    selection.add_argument("--general-only", action="store_true")
    selection.add_argument("--instruction-path", action="append")
    draft_check.add_argument("--reference", action="append", default=[])
    for command_name in ("draft-list-update", "draft-list-recover"):
        draft_list = task_commands.add_parser(command_name)
        draft_list.add_argument("--stdin", action="store_true", required=True)
        draft_list.add_argument("--expected-revision", type=int, required=True)

    task_validate = task_commands.add_parser("validate")
    for command_name in ("draft-source-update", "draft-source-recover"):
        source_update = task_commands.add_parser(command_name)
        source_update.add_argument("--stdin", action="store_true", required=True)
        source_update.add_argument("--requirement-id", required=True)
        source_update.add_argument("--expected-revision", type=int, required=True)
        source_update.add_argument("--plan-path", required=True)
        source_update.add_argument("--user-config-root", required=True)
        source_update.add_argument("--skill-root", action="append", default=[])
    for command_name in ("draft-assemble", "draft-create"):
        assembly = task_commands.add_parser(command_name)
        assembly.add_argument("--stdin", action="store_true", required=True)
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
    task_source.add_argument("--stdin", action="store_true")
    task_validate.add_argument("--task-path")

    for command_name in ("create", "recover-create"):
        task_write = task_commands.add_parser(command_name)
        task_write.add_argument("--user-config-root", required=True)
        task_write.add_argument("--skill-root", action="append", default=[])
        task_write.add_argument("--stdin", action="store_true", required=True)
        task_write.add_argument("--plan-path", required=True)
        task_write.add_argument("--task-path", required=True)
        task_write.add_argument("--execution-dir", required=True)


def run_task(
    arguments: argparse.Namespace,
    project_root: Path,
    input_stream: TextIO,
) -> dict[str, object]:
    if arguments.task_command in {"spec-validate", "spec-update", "spec-recover"}:
        return update_specification(
            input_stream.read().encode("utf-8"), project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            operation={"spec-validate": "validate", "spec-update": "apply", "spec-recover": "recover"}[arguments.task_command],
            approved_sha256=getattr(arguments, "approved_sha256", None),
        )
    if arguments.task_command in {"draft-list-update", "draft-list-recover"}:
        request = strict_keys(
            parse_json_contract(input_stream.read().encode("utf-8"), source="stdin"),
            location="draft_list_update", required={"index", "reason"},
        )
        return update_task_planning_list(
            project_root, request["index"], expected_revision=arguments.expected_revision,
            reason=request["reason"], recover=arguments.task_command == "draft-list-recover",
        )
    if arguments.task_command == "draft-read":
        if arguments.task_id is not None:
            return read_task_draft(project_root, arguments.requirement_id, arguments.task_id)
        return read_task_planning_index(project_root, arguments.requirement_id)
    if arguments.task_command in {"draft-init", "draft-save", "draft-recover"}:
        payload = parse_json_contract(input_stream.read().encode("utf-8"), source="stdin")
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
    if arguments.task_command in {"draft-source-update", "draft-source-recover"}:
        return update_task_draft_sources(
            project_root, arguments.requirement_id,
            parse_json_contract(input_stream.read().encode("utf-8"), source="stdin"),
            expected_revision=arguments.expected_revision, plan_path=arguments.plan_path,
            user_config_root=arguments.user_config_root, skill_roots=skill_roots,
            recover=arguments.task_command == "draft-source-recover",
        )
    if arguments.task_command in {"draft-assemble", "draft-create"}:
        metadata = parse_json_contract(input_stream.read().encode("utf-8"), source="stdin")
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
            selected_paths=arguments.instruction_path or [], reference_names=arguments.reference,
        )
    if arguments.task_command in {"create", "recover-create"}:
        operation = (
            create_task_artifacts
            if arguments.task_command == "create"
            else recover_task_create
        )
        return operation(
            input_stream.read().encode("utf-8"),
            source="stdin",
            raw_plan_path=arguments.plan_path,
            raw_task_path=arguments.task_path,
            raw_execution_dir=arguments.execution_dir,
            project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=skill_roots,
        )
    if arguments.stdin:
        if not arguments.task_path:
            raise WorkError(
                ExitCode.CLI_USAGE,
                "task_path_required",
                "--task-path is required with --stdin.",
            )
        return validate_task_json_contract(
            input_stream.read().encode("utf-8"),
            source="stdin",
            actual_task_path=arguments.task_path,
            project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=skill_roots,
        )
    if arguments.task_path:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "unexpected_task_path",
            "--task-path is only valid with --stdin.",
        )
    return validate_task_file(
        project_root,
        arguments.user_config_root,
        arguments.path,
        skill_roots=skill_roots,
    )
