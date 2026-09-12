from __future__ import annotations

import argparse
from pathlib import Path

from ..contracts.task import validate_task_contract
from ..foundation.fingerprint import read_raw
from ..foundation.paths import normalize_relative_path
from ..execution.attempt_close import close_attempt
from ..execution.attempt_start import recover_attempt_start, start_attempt
from ..execution.command_correction import record_command_correction
from ..execution.correction import create_correction
from ..execution.preflight import execute_preflight
from ..execution.record_begin import begin_record
from ..execution.record_finish import finish_record
from ..execution.recovery import recover_execution
from ..execution.worktree import inspect_execute_worktree
from ..foundation.spec_update import require_no_spec_update, state_writer, storage_path
from ..skills.catalog import parse_skill_root
from ..foundation.cli_io import FileInput
from . import SubparserRegistry


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
        execute_command.add_argument(
            "--confirmed-input", action="append", default=[]
        )

    for command_name in ("attempt-start", "recover-attempt-start"):
        execute_command = execute_commands.add_parser(command_name)
        _add_execution_context_arguments(execute_command)
        execute_command.add_argument(
            "--confirmed-input", action="append", default=[]
        )
        execute_command.add_argument("--input-file", required=True)

    record_begin = execute_commands.add_parser("record-begin")
    _add_execution_context_arguments(record_begin)
    record_begin.add_argument("--record-id", required=True)

    for command_name in (
        "command-correction",
        "record-finish",
        "attempt-close",
        "correction-create",
        "recover",
    ):
        execute_command = execute_commands.add_parser(command_name)
        _add_execution_context_arguments(execute_command)
        execute_command.add_argument("--input-file", required=True)


def run_execute(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    if arguments.execute_command not in {"preflight", "worktree"}:
        task_path = storage_path(project_root, arguments.task_path)
        if task_path.is_file():
            validate_task_contract(
                read_raw(task_path), source=str(task_path),
                actual_task_path=normalize_relative_path(arguments.task_path, field="task_path"),
                project_root=project_root, user_config_root=arguments.user_config_root,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
                validate_file_state=False,
            )
        directory = storage_path(project_root, arguments.execution_dir)
        # Missing artifacts are reported by the existing command validator.
        if directory.is_dir():
            with state_writer(project_root, arguments.execution_dir):
                return _run_execute(arguments, project_root, request)
    return _run_execute(arguments, project_root, request)


def _run_execute(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    require_no_spec_update(project_root, arguments.execution_dir)
    common = {
        "project_root": project_root,
        "user_config_root": arguments.user_config_root,
        "raw_task_path": arguments.task_path,
        "raw_execution_dir": arguments.execution_dir,
        "task_id": arguments.task_id,
        "skill_roots": [parse_skill_root(root) for root in arguments.skill_root],
    }

    if arguments.execute_command == "record-begin":
        return begin_record(
            **common,
            base_record_id=arguments.record_id,
        )

    file_operations = {
        "command-correction": record_command_correction,
        "record-finish": finish_record,
        "attempt-close": close_attempt,
        "correction-create": create_correction,
        "recover": recover_execution,
    }
    if arguments.execute_command in file_operations:
        return file_operations[arguments.execute_command](
            request.raw,
            source=request.source,
            **common,
        )

    common["confirmed_inputs"] = arguments.confirmed_input
    if arguments.execute_command == "preflight":
        return execute_preflight(**common)
    if arguments.execute_command == "worktree":
        return inspect_execute_worktree(**common)

    operation = (
        start_attempt
        if arguments.execute_command == "attempt-start"
        else recover_attempt_start
    )
    return operation(
        request.raw,
        source=request.source,
        **common,
    )
