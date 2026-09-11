from __future__ import annotations

import argparse
from pathlib import Path
from typing import TextIO

from ..artifacts.progress import preview_progress, read_progress, save_progress
from ..foundation.markdown import parse_json_contract
from . import SubparserRegistry


def register_progress_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("progress")
    operations = parser.add_subparsers(dest="progress_command", required=True)
    reader = operations.add_parser("read")
    reader.add_argument("--requirement-id", required=True)
    reader.add_argument("--mode", choices=("plan", "task"), required=True)
    for name in ("validate", "save"):
        operation = operations.add_parser(name)
        operation.add_argument("--stdin", action="store_true", required=True)
        operation.add_argument("--expected-revision", type=int, required=True)
        if name == "save":
            operation.add_argument("--approved-sha256", required=True)


def run_progress(
    arguments: argparse.Namespace, project_root: Path, input_stream: TextIO,
) -> dict[str, object]:
    if arguments.progress_command == "read":
        return read_progress(project_root, arguments.requirement_id, arguments.mode)
    value = parse_json_contract(input_stream.read().encode("utf-8"), source="stdin")
    if arguments.progress_command == "save":
        return save_progress(
            project_root, value, expected_revision=arguments.expected_revision,
            approved_sha256=arguments.approved_sha256,
        )
    return preview_progress(project_root, value, expected_revision=arguments.expected_revision)
