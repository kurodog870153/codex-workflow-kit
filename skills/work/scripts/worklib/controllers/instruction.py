from __future__ import annotations

import argparse
from pathlib import Path

from . import SubparserRegistry
from ..business_services.instruction import catalog, instruction_root, load, resolve, select


def register_instruction_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("instructions")
    subcommands = parser.add_subparsers(dest="instructions_command", required=True)
    catalog_parser = subcommands.add_parser("catalog")
    catalog_parser.add_argument("--mode", choices=("plan", "task", "execute", "all"), required=True)
    resolve_parser = subcommands.add_parser("resolve")
    resolve_parser.add_argument("--mode", choices=("plan", "task", "execute"), required=True)
    resolve_parser.add_argument("paths", nargs="*")
    for name in ("load", "select"):
        source = subcommands.add_parser(name)
        source.add_argument("--mode", choices=("plan", "task", "execute"), required=True)
        source.add_argument("--reference", action="append", default=[])
        source.add_argument("paths", nargs="*")


def run_instructions(arguments: argparse.Namespace, project_root: Path) -> dict[str, object]:
    skill_root = instruction_root()
    if arguments.instructions_command == "catalog":
        return catalog(skill_root, arguments.mode)
    if arguments.instructions_command == "load":
        return load(skill_root, arguments.mode, arguments.paths, arguments.reference)
    if arguments.instructions_command == "select":
        return select(skill_root, arguments.mode, arguments.paths, arguments.reference)
    result = resolve(skill_root, arguments.mode, arguments.paths)
    result["project_root"] = str(project_root)
    return result
