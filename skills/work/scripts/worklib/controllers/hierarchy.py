from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from . import SubparserRegistry
from ..business_services.hierarchy import (
    build_hierarchy, build_hierarchy_selection_json,
    validate_hierarchy_selection_json,
)


def register_hierarchy_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("hierarchy")
    subcommands = parser.add_subparsers(dest="hierarchy_command", required=True)
    resolve = subcommands.add_parser("resolve")
    resolve.add_argument("--work-directory", choices=("plan", "task", "execute"), required=True)
    resolve.add_argument("paths", nargs="*")
    for name in ("selection-build", "selection-validate"):
        selection = subcommands.add_parser(name)
        selection.add_argument("--input-file", required=True)


def run_hierarchy(arguments: argparse.Namespace, project_root: Path, request: Any) -> dict[str, object]:
    if arguments.hierarchy_command == "resolve":
        result = build_hierarchy(arguments.work_directory, arguments.paths).as_dict()
        result["project_root"] = str(project_root)
        return result
    operation = build_hierarchy_selection_json if arguments.hierarchy_command == "selection-build" else validate_hierarchy_selection_json
    return operation(request.raw, skill_root=Path(__file__).resolve().parents[3])
