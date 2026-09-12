from __future__ import annotations

import argparse
from pathlib import Path

from ..contracts.handoff import (
    render_handoff_json_contract,
    validate_handoff_json_contract,
)
from ..foundation.cli_io import FileInput
from . import SubparserRegistry


def register_handoff_commands(commands: SubparserRegistry) -> None:
    handoff_parser = commands.add_parser("handoff")
    handoff_commands = handoff_parser.add_subparsers(
        dest="handoff_command", required=True
    )
    for command_name in ("validate", "render"):
        handoff_command = handoff_commands.add_parser(command_name)
        handoff_command.add_argument("--input-file", required=True)


def run_handoff(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    operation = (
        validate_handoff_json_contract
        if arguments.handoff_command == "validate"
        else render_handoff_json_contract
    )
    return operation(
        request.raw,
        source=request.source,
        project_root=project_root,
    )
