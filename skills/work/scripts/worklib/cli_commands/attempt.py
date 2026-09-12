from __future__ import annotations

import argparse
from pathlib import Path

from ..contracts.attempt import (
    render_attempt_json_contract,
    validate_attempt_file,
    validate_attempt_json_contract,
)
from ..foundation.cli_io import FileInput
from . import SubparserRegistry


def register_attempt_commands(commands: SubparserRegistry) -> None:
    attempt_parser = commands.add_parser("attempt")
    attempt_commands = attempt_parser.add_subparsers(
        dest="attempt_command", required=True
    )

    attempt_validate = attempt_commands.add_parser("validate")
    attempt_source = attempt_validate.add_mutually_exclusive_group(required=True)
    attempt_source.add_argument("--path")
    attempt_source.add_argument("--input-file")

    attempt_render = attempt_commands.add_parser("render")
    attempt_render.add_argument("--input-file", required=True)


def run_attempt(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    if arguments.attempt_command == "render":
        return render_attempt_json_contract(
            request.raw,
            source=request.source,
            project_root=project_root,
        )
    if arguments.input_file:
        return validate_attempt_json_contract(
            request.raw,
            source=request.source,
            project_root=project_root,
        )
    return validate_attempt_file(project_root, arguments.path)
