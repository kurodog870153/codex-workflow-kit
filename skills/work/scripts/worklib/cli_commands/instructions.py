from __future__ import annotations

import argparse
from pathlib import Path

from ..foundation.runtime import installed_work_root
from ..instructions.catalog import (
    build_cross_mode_instruction_catalog,
    build_instruction_catalog,
    resolve_instruction_hierarchy,
)
from ..instructions.selection import build_instruction_selection
from ..instructions.sources import load_instruction_sources
from . import SubparserRegistry


def register_instruction_commands(commands: SubparserRegistry) -> None:
    instructions_parser = commands.add_parser("instructions")
    instructions_commands = instructions_parser.add_subparsers(
        dest="instructions_command", required=True
    )

    instructions_catalog = instructions_commands.add_parser("catalog")
    instructions_catalog.add_argument(
        "--mode", choices=("plan", "task", "execute", "all"), required=True
    )

    instructions_resolve = instructions_commands.add_parser("resolve")
    instructions_resolve.add_argument(
        "--mode", choices=("plan", "task", "execute"), required=True
    )
    instructions_resolve.add_argument("paths", nargs="*")

    for command_name in ("load", "select"):
        instructions_source = instructions_commands.add_parser(command_name)
        instructions_source.add_argument(
            "--mode", choices=("plan", "task", "execute"), required=True
        )
        instructions_source.add_argument(
            "--reference", action="append", default=[]
        )
        instructions_source.add_argument("paths", nargs="*")


def run_instructions(
    arguments: argparse.Namespace,
    project_root: Path,
) -> dict[str, object]:
    skill_root = installed_work_root()
    if arguments.instructions_command == "catalog":
        if arguments.mode == "all":
            return build_cross_mode_instruction_catalog(skill_root).as_dict()
        return build_instruction_catalog(skill_root, arguments.mode).as_dict()
    if arguments.instructions_command == "load":
        return load_instruction_sources(
            skill_root,
            arguments.mode,
            arguments.paths,
            arguments.reference,
        ).as_dict()
    if arguments.instructions_command == "select":
        return {
            "schema": "work-instruction-selection/v1",
            "mode": arguments.mode,
            "instruction_selection": build_instruction_selection(
                skill_root=skill_root,
                mode=arguments.mode,
                selected_paths=arguments.paths,
                reference_names=arguments.reference,
            ),
        }
    result = resolve_instruction_hierarchy(
        skill_root,
        arguments.mode,
        arguments.paths,
    ).as_dict()
    result["project_root"] = str(project_root)
    return result
