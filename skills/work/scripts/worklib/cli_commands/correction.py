from __future__ import annotations

import argparse
from pathlib import Path
from typing import TextIO

from ..contracts.correction import (
    render_correction_json_contract,
    validate_correction_file,
    validate_correction_json_contract,
)
from . import SubparserRegistry


def register_correction_commands(commands: SubparserRegistry) -> None:
    correction_parser = commands.add_parser("correction")
    correction_commands = correction_parser.add_subparsers(
        dest="correction_command", required=True
    )

    correction_validate = correction_commands.add_parser("validate")
    correction_source = correction_validate.add_mutually_exclusive_group(required=True)
    correction_source.add_argument("--path")
    correction_source.add_argument("--stdin", action="store_true")

    correction_render = correction_commands.add_parser("render")
    correction_render.add_argument("--stdin", action="store_true", required=True)


def run_correction(
    arguments: argparse.Namespace,
    project_root: Path,
    input_stream: TextIO,
) -> dict[str, object]:
    if arguments.correction_command == "render":
        return render_correction_json_contract(
            input_stream.read().encode("utf-8"), source="stdin"
        )
    if arguments.stdin:
        return validate_correction_json_contract(
            input_stream.read().encode("utf-8"), source="stdin"
        )
    return validate_correction_file(project_root, arguments.path)
