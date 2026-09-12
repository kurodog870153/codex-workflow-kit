from __future__ import annotations

import argparse
from pathlib import Path

from ..artifacts.plan import create_plan_file
from ..contracts.plan import validate_plan_file, validate_plan_json_contract
from ..foundation.errors import ExitCode, WorkError
from ..skills.catalog import parse_skill_root
from ..foundation.cli_io import FileInput
from . import SubparserRegistry


def register_plan_commands(commands: SubparserRegistry) -> None:
    plan_parser = commands.add_parser("plan")
    plan_commands = plan_parser.add_subparsers(dest="plan_command", required=True)

    plan_validate = plan_commands.add_parser("validate")
    plan_validate.add_argument("--user-config-root", required=True)
    plan_validate.add_argument("--skill-root", action="append", default=[])
    plan_source = plan_validate.add_mutually_exclusive_group(required=True)
    plan_source.add_argument("--path")
    plan_source.add_argument("--input-file")
    plan_validate.add_argument("--plan-path")

    plan_create = plan_commands.add_parser("create")
    plan_create.add_argument("--user-config-root", required=True)
    plan_create.add_argument("--skill-root", action="append", default=[])
    plan_create.add_argument("--input-file", required=True)
    plan_create.add_argument("--plan-path", required=True)


def run_plan(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    skill_roots = [parse_skill_root(root) for root in arguments.skill_root]
    if arguments.plan_command == "create":
        return create_plan_file(
            request.raw,
            source=request.source,
            raw_plan_path=arguments.plan_path,
            project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=skill_roots,
        )
    if arguments.input_file:
        if not arguments.plan_path:
            raise WorkError(
                ExitCode.CLI_USAGE,
                "plan_path_required",
                "--plan-path is required with --input-file.",
            )
        return validate_plan_json_contract(
            request.raw,
            source=request.source,
            actual_plan_path=arguments.plan_path,
            project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=skill_roots,
        )
    if arguments.plan_path:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "unexpected_plan_path",
            "--plan-path is only valid with --input-file.",
        )
    return validate_plan_file(
        project_root,
        arguments.user_config_root,
        arguments.path,
        skill_roots=skill_roots,
    )
