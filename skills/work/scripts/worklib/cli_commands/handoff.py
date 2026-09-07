from __future__ import annotations

import argparse
from pathlib import Path
from typing import TextIO

from ..contracts.handoff import (
    render_handoff_json_contract,
    validate_handoff_json_contract,
)
from . import SubparserRegistry


def register_handoff_commands(commands: SubparserRegistry) -> None:
    handoff_parser = commands.add_parser("handoff")
    handoff_commands = handoff_parser.add_subparsers(
        dest="handoff_command", required=True
    )
    for command_name in ("validate", "render"):
        handoff_command = handoff_commands.add_parser(command_name)
        handoff_command.add_argument("--stdin", action="store_true", required=True)


def run_handoff(
    arguments: argparse.Namespace,
    project_root: Path,
    input_stream: TextIO,
) -> dict[str, object]:
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
