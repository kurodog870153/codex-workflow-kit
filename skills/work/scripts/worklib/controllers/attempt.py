from __future__ import annotations

import argparse
from pathlib import Path

from . import RequestInput, SubparserRegistry


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
    request: RequestInput | None,
) -> dict[str, object]:
    from ..business_services.attempt import handle_attempt_request

    return handle_attempt_request(arguments, project_root, request)
