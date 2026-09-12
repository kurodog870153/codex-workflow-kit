from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Sequence, TextIO

from .cli_commands.attempt import register_attempt_commands, run_attempt
from .cli_commands.correction import register_correction_commands, run_correction
from .cli_commands.execute import register_execute_commands, run_execute
from .cli_commands.handoff import register_handoff_commands, run_handoff
from .cli_commands.hierarchy import register_hierarchy_commands, run_hierarchy
from .cli_commands.instructions import (
    register_instruction_commands,
    run_instructions,
)
from .cli_commands.plan import register_plan_commands, run_plan
from .cli_commands.progress import register_progress_commands, run_progress
from .cli_commands.skills import register_skill_commands, run_skills
from .cli_commands.task import register_task_commands, run_task
from .foundation.cli_io import FileInput, error_response, read_input_file, success_response
from .foundation.errors import ExitCode, WorkError
from .foundation.fingerprint import fingerprint_file
from .foundation.jsonio import write_json
from .foundation.paths import (
    default_artifact_paths,
    resolve_project_relative_path,
    resolve_root,
)


class HelpRequested(Exception):
    def __init__(self, text: str) -> None:
        self.text = text


class JsonHelpAction(argparse.Action):
    def __call__(self, parser, namespace, values, option_string=None) -> None:
        raise HelpRequested(parser.format_help())


class WorkArgumentParser(argparse.ArgumentParser):
    def __init__(self, *args, **kwargs) -> None:
        kwargs["add_help"] = False
        kwargs["allow_abbrev"] = False
        super().__init__(*args, **kwargs)
        self.add_argument(
            "-h", "--help", action=JsonHelpAction, nargs=0,
            help="Return command help in a JSON response.",
        )

    def error(self, message: str) -> None:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "cli_usage_error",
            "The CLI arguments are invalid.",
            {"reason": message},
        )


def build_parser() -> WorkArgumentParser:
    parser = WorkArgumentParser(prog="work.py")
    parser.add_argument("--project-root", required=True)
    commands = parser.add_subparsers(dest="command", required=True)

    paths_parser = commands.add_parser("paths")
    paths_commands = paths_parser.add_subparsers(dest="paths_command", required=True)
    paths_resolve = paths_commands.add_parser("resolve")
    paths_resolve.add_argument("--requirement-id", required=True)

    register_hierarchy_commands(commands)

    register_instruction_commands(commands)

    fingerprint_parser = commands.add_parser("fingerprint")
    fingerprint_commands = fingerprint_parser.add_subparsers(
        dest="fingerprint_command", required=True
    )
    fingerprint_text = fingerprint_commands.add_parser("text")
    fingerprint_text.add_argument("--path", required=True)

    register_skill_commands(commands)

    register_handoff_commands(commands)

    register_attempt_commands(commands)

    register_correction_commands(commands)

    register_plan_commands(commands)

    register_progress_commands(commands)

    register_task_commands(commands)

    register_execute_commands(commands)
    return parser


def _run(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    if arguments.command == "paths":
        return {
            "schema": "work-paths/v1",
            "project_root": str(project_root),
            "requirement_id": arguments.requirement_id,
            "paths": default_artifact_paths(project_root, arguments.requirement_id),
        }

    if arguments.command == "hierarchy":
        return run_hierarchy(arguments, project_root, request)

    if arguments.command == "instructions":
        return run_instructions(arguments, project_root)

    if arguments.command == "fingerprint":
        normalized, path = resolve_project_relative_path(
            project_root,
            arguments.path,
            field="path",
        )
        result: dict[str, object] = {
            "schema": "work-fingerprint/v1",
            "path": normalized,
        }
        result.update(fingerprint_file(path))
        return result

    if arguments.command == "skills":
        return run_skills(arguments, request)

    if arguments.command == "handoff":
        return run_handoff(arguments, project_root, request)

    if arguments.command == "attempt":
        return run_attempt(arguments, project_root, request)

    if arguments.command == "correction":
        return run_correction(arguments, project_root, request)

    if arguments.command == "plan":
        return run_plan(arguments, project_root, request)

    if arguments.command == "progress":
        return run_progress(arguments, project_root, request)

    if arguments.command == "task":
        return run_task(arguments, project_root, request)

    if arguments.command == "execute":
        return run_execute(arguments, project_root, request)

    raise WorkError(
        ExitCode.INTERNAL_ERROR,
        "unreachable_command",
        "The parsed command could not be dispatched.",
    )


def main(
    argv: Sequence[str] | None = None,
    *,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
) -> int:
    # stderr is reserved for optional diagnostics; every result goes to stdout.
    output = stdout if stdout is not None else sys.stdout
    try:
        tokens = list(sys.argv[1:] if argv is None else argv)
        if any(token == "--stdin" or token.startswith("--stdin=") for token in tokens):
            raise WorkError(
                ExitCode.CLI_USAGE,
                "stdin_removed",
                "Write the JSON request to a UTF-8 file and use --input-file <path>.",
                {"replacement": "--input-file"},
            )
        arguments = build_parser().parse_args(tokens)
        project_root = resolve_root(arguments.project_root, label="project root")
        path = getattr(arguments, "input_file", None)
        request = read_input_file(path) if path is not None else None
        result = _run(arguments, project_root, request)
        # Preserve the existing data order independently of the fixed envelope.
        preserve_order = (
            arguments.command == "attempt" and arguments.attempt_command == "render"
        ) or (
            arguments.command == "correction" and arguments.correction_command == "render"
        )
        write_json(
            output, success_response(result, preserve_order=preserve_order), sort_keys=False,
        )
        return int(ExitCode.SUCCESS)
    except HelpRequested as help_result:
        write_json(
            output, success_response({"help": help_result.text}), sort_keys=False,
        )
        return int(ExitCode.SUCCESS)
    except WorkError as error:
        write_json(output, error_response(error), sort_keys=False)
        return int(error.exit_code)
    except Exception:
        error = WorkError(
            ExitCode.INTERNAL_ERROR,
            "internal_error",
            "An unexpected internal error occurred.",
        )
        write_json(output, error_response(error), sort_keys=False)
        return int(error.exit_code)
