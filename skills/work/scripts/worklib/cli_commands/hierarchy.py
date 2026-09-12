from __future__ import annotations

import argparse
from pathlib import Path

from ..foundation.hierarchy import build_hierarchy
from ..foundation.runtime import installed_work_root
from ..hierarchy.selection import (
    build_hierarchy_selection_json,
    validate_hierarchy_selection_json,
)
from ..foundation.cli_io import FileInput
from . import SubparserRegistry


def register_hierarchy_commands(commands: SubparserRegistry) -> None:
    hierarchy_parser = commands.add_parser("hierarchy")
    hierarchy_commands = hierarchy_parser.add_subparsers(
        dest="hierarchy_command", required=True
    )

    hierarchy_resolve = hierarchy_commands.add_parser("resolve")
    hierarchy_resolve.add_argument(
        "--work-directory", choices=("plan", "task", "execute"), required=True
    )
    hierarchy_resolve.add_argument("paths", nargs="*")

    for command_name in ("selection-build", "selection-validate"):
        hierarchy_selection = hierarchy_commands.add_parser(command_name)
        hierarchy_selection.add_argument(
            "--input-file", required=True
        )


def run_hierarchy(
    arguments: argparse.Namespace,
    project_root: Path,
    request: FileInput | None,
) -> dict[str, object]:
    if arguments.hierarchy_command == "resolve":
        result = build_hierarchy(arguments.work_directory, arguments.paths).as_dict()
        result["project_root"] = str(project_root)
        return result

    skill_root = installed_work_root()
    operation = (
        build_hierarchy_selection_json
        if arguments.hierarchy_command == "selection-build"
        else validate_hierarchy_selection_json
    )
    return operation(
        request.raw,
        skill_root=skill_root,
    )
