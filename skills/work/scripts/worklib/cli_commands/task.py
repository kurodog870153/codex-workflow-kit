from __future__ import annotations

import argparse
from pathlib import Path
from typing import TextIO

from ..artifacts.task import create_task_artifacts, recover_task_create
from ..contracts.task import validate_task_file, validate_task_json_contract
from ..foundation.errors import ExitCode, WorkError
from ..skills.catalog import parse_skill_root
from . import SubparserRegistry


def register_task_commands(commands: SubparserRegistry) -> None:
    task_parser = commands.add_parser("task")
    task_commands = task_parser.add_subparsers(dest="task_command", required=True)

    task_validate = task_commands.add_parser("validate")
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
    skill_roots = [parse_skill_root(root) for root in arguments.skill_root]
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
