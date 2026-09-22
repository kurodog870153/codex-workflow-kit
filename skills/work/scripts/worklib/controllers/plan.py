from __future__ import annotations

import argparse
from pathlib import Path

from ..business_services.plan import create_plan_file, parse_roots, prepare_initial_plan, prepare_semantic_plan, validate_plan_request
from . import SubparserRegistry


def register_plan_commands(commands: SubparserRegistry) -> None:
    plan_parser = commands.add_parser("plan")
    plan_commands = plan_parser.add_subparsers(dest="plan_command", required=True)
    prepare = plan_commands.add_parser("prepare")
    prepare.add_argument("--input-file", required=True)
    prepare.add_argument("--user-config-root", required=True)
    prepare.add_argument("--skill-root", action="append", default=[])
    prepare.add_argument("--output-file")
    semantic = plan_commands.add_parser("semantic-prepare")
    semantic.add_argument("--input-file", required=True)
    semantic.add_argument("--user-config-root", required=True)
    semantic.add_argument("--skill-root", action="append", default=[])
    semantic.add_argument("--output-file")

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
    request: object | None,
) -> dict[str, object]:
    skill_roots = parse_roots(arguments.skill_root)
    raw = getattr(request, "raw", None)
    source = getattr(request, "source", None)
    if arguments.plan_command == "prepare":
        return prepare_initial_plan(raw, source=source, project_root=project_root,
                                    user_config_root=arguments.user_config_root, skill_roots=skill_roots,
                                    output_file=arguments.output_file)
    if arguments.plan_command == "semantic-prepare":
        return prepare_semantic_plan(raw, source=source, project_root=project_root,
                                     user_config_root=arguments.user_config_root, skill_roots=skill_roots,
                                     output_file=arguments.output_file)
    if arguments.plan_command == "create":
        return create_plan_file(
            raw,
            source=source,
            raw_plan_path=arguments.plan_path,
            project_root=project_root,
            user_config_root=arguments.user_config_root,
            skill_roots=skill_roots,
        )
    if arguments.input_file:
        pass
    return validate_plan_request(input_file=bool(arguments.input_file), plan_path=arguments.plan_path,
        path=arguments.path, raw=raw, source=source, project_root=project_root,
        user_config_root=arguments.user_config_root, skill_roots=skill_roots)
