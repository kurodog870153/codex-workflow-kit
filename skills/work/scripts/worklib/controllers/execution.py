from __future__ import annotations

import argparse
from pathlib import Path

from . import RequestInput, SubparserRegistry
from ..orchestration.execution import execution_service


def _add_execution_context_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--user-config-root", required=True)
    parser.add_argument("--task-path", required=True)
    parser.add_argument("--execution-dir", required=True)
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--skill-root", action="append", default=[])


def register_execute_commands(commands: SubparserRegistry) -> None:
    execute_parser = commands.add_parser("execute")
    execute_commands = execute_parser.add_subparsers(
        dest="execute_command", required=True
    )

    for command_name in ("preflight", "worktree"):
        execute_command = execute_commands.add_parser(command_name)
        _add_execution_context_arguments(execute_command)
        execute_command.add_argument("--confirmed-input", action="append", default=[])

    for command_name in ("attempt-start", "attempt-start-prepare", "recover-attempt-start"):
        execute_command = execute_commands.add_parser(command_name)
        _add_execution_context_arguments(execute_command)
        execute_command.add_argument("--confirmed-input", action="append", default=[])
        execute_command.add_argument("--input-file", required=True)

    record_begin = execute_commands.add_parser("record-begin")
    _add_execution_context_arguments(record_begin)
    record_begin.add_argument("--record-id", required=True)
    record_begin.add_argument("--authorization-evidence")

    for command_name in (
        "command-correction",
        "record-finish",
        "attempt-close",
        "correction-create",
        "recover",
        "recovery-prepare",
        "command-prepare",
        "deviation-prepare-semantic",
        "deviation-record",
        "command-run",
    ):
        execute_command = execute_commands.add_parser(command_name)
        _add_execution_context_arguments(execute_command)
        execute_command.add_argument("--input-file", required=True)
        if command_name in {"command-run", "deviation-record"}:
            execute_command.add_argument("--approved-sha256", required=True)
        if command_name == "deviation-record":
            execute_command.add_argument("--authorization-evidence", required=True)


def run_execute(
    arguments: argparse.Namespace,
    project_root: Path,
    request: RequestInput | None,
) -> dict[str, object]:
    return execution_service.execute(
        arguments.execute_command,
        project_root=project_root,
        user_config_root=arguments.user_config_root,
        raw_task_path=arguments.task_path,
        raw_execution_dir=arguments.execution_dir,
        task_id=arguments.task_id,
        skill_roots=arguments.skill_root,
        raw_request=request.raw if request is not None else None,
        source=request.source if request is not None else None,
        confirmed_inputs=getattr(arguments, "confirmed_input", None),
        base_record_id=getattr(arguments, "record_id", None),
        approved_sha256=getattr(arguments, "approved_sha256", None),
        authorization_evidence=getattr(arguments, "authorization_evidence", None),
    )
