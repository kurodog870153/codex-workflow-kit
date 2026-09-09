from __future__ import annotations

import argparse
from pathlib import Path
from typing import TextIO

from ..artifacts.handoff import build_plan_to_task_handoff, build_task_to_execute_handoff, build_task_to_plan_handoff
from ..artifacts.handoff import build_execute_return_handoff, build_preflight_return_handoff
from ..artifacts.handoff import verify_plan_to_task_handoff, verify_task_to_execute_handoff
from ..contracts.handoff import (
    render_handoff_json_contract,
    validate_handoff_json_contract,
)
from ..foundation.markdown import parse_json_contract
from ..skills.catalog import parse_skill_root
from . import SubparserRegistry


def register_handoff_commands(commands: SubparserRegistry) -> None:
    handoff_parser = commands.add_parser("handoff")
    handoff_commands = handoff_parser.add_subparsers(
        dest="handoff_command", required=True
    )
    for command_name in ("validate", "render"):
        handoff_command = handoff_commands.add_parser(command_name)
        handoff_command.add_argument("--stdin", action="store_true", required=True)
    for command_name in ("verify-plan-to-task", "verify-task-to-execute"):
        verify = handoff_commands.add_parser(command_name, help="Verify incoming handoff identity against the selected current source.")
        verify.add_argument("--stdin", action="store_true", required=True)
        verify.add_argument("--user-config-root", required=True)
        verify.add_argument("--skill-root", action="append", default=[])
        if command_name == "verify-plan-to-task":
            verify.add_argument("--plan-path", required=True)
        else:
            verify.add_argument("--task-path", required=True)
            verify.add_argument("--task-id", required=True)
    for command_name in ("build-plan-to-task", "build-task-to-execute", "build-task-to-plan", "build-execute-to-task", "build-execute-to-plan"):
        build = handoff_commands.add_parser(command_name, help="Build a handoff from a validated formal artifact and semantic input.")
        build.add_argument("--stdin", action="store_true", required=True)
        build.add_argument("--user-config-root", required=True)
        build.add_argument("--skill-root", action="append", default=[])
        if command_name == "build-plan-to-task":
            build.add_argument("--plan-path", required=True)
        else:
            build.add_argument("--task-path", required=True)
            build.add_argument("--task-id", required=command_name != "build-task-to-plan")
            if command_name.startswith("build-execute-"):
                context = build.add_mutually_exclusive_group(required=True)
                context.add_argument("--attempt-id")
                context.add_argument("--preflight", action="store_true")


def run_handoff(
    arguments: argparse.Namespace,
    project_root: Path,
    input_stream: TextIO,
) -> dict[str, object]:
    if arguments.handoff_command == "verify-task-to-execute":
        return verify_task_to_execute_handoff(
            project_root, parse_json_contract(input_stream.read().encode("utf-8"), source="stdin"),
            task_path=arguments.task_path, task_id=arguments.task_id, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.handoff_command == "verify-plan-to-task":
        return verify_plan_to_task_handoff(
            project_root, parse_json_contract(input_stream.read().encode("utf-8"), source="stdin"),
            plan_path=arguments.plan_path, user_config_root=arguments.user_config_root,
            skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
        )
    if arguments.handoff_command in {"build-plan-to-task", "build-task-to-execute", "build-task-to-plan", "build-execute-to-task", "build-execute-to-plan"}:
        request = parse_json_contract(input_stream.read().encode("utf-8"), source="stdin")
        common = {"user_config_root": arguments.user_config_root, "skill_roots": [parse_skill_root(root) for root in arguments.skill_root]}
        if arguments.handoff_command.startswith("build-execute-"):
            if arguments.preflight:
                return build_preflight_return_handoff(
                    project_root, request, direction=arguments.handoff_command.removeprefix("build-").replace("-", "_"),
                    task_path=arguments.task_path, task_id=arguments.task_id, **common,
                )
            return build_execute_return_handoff(
                project_root, request, direction=arguments.handoff_command.removeprefix("build-").replace("-", "_"),
                task_path=arguments.task_path, task_id=arguments.task_id, attempt_id=arguments.attempt_id, **common,
            )
        if arguments.handoff_command == "build-plan-to-task":
            return build_plan_to_task_handoff(project_root, request, plan_path=arguments.plan_path, **common)
        operation = build_task_to_plan_handoff if arguments.handoff_command == "build-task-to-plan" else build_task_to_execute_handoff
        return operation(project_root, request, task_path=arguments.task_path, task_id=arguments.task_id, **common)
    operation = (
        validate_handoff_json_contract
        if arguments.handoff_command == "validate"
        else render_handoff_json_contract
    )
    return operation(
        input_stream.read().encode("utf-8"),
        source="stdin",
        project_root=project_root,
    )
