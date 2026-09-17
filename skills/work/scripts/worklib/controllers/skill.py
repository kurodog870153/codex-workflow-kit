from __future__ import annotations

import argparse

from . import SubparserRegistry
from ..infrastructure.cli_io import FileInput
from ..services.skill import build_selection, catalog, snapshot, validate_selection
from ..services.skill_catalog import parse_skill_root


def register_skill_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("skills")
    subcommands = parser.add_subparsers(dest="skills_command", required=True)
    catalog_parser = subcommands.add_parser("catalog")
    catalog_parser.add_argument("--root", action="append", required=True)
    catalog_parser.add_argument("--disabled-source", action="append", default=[])
    snapshot_parser = subcommands.add_parser("snapshot")
    snapshot_parser.add_argument("--root", required=True)
    snapshot_parser.add_argument("--source", required=True)
    validate_parser = subcommands.add_parser("selection-validate")
    validate_parser.add_argument("--root", action="append", required=True)
    validate_parser.add_argument("--input-file", required=True)
    build_parser = subcommands.add_parser("selection-build")
    build_parser.add_argument("--root", action="append", default=[])
    build_parser.add_argument("--input-file", required=True)


def run_skills(arguments: argparse.Namespace, request: FileInput | None) -> dict[str, object]:
    if arguments.skills_command == "selection-build":
        return build_selection(request.raw, source=request.source, roots=[parse_skill_root(root) for root in arguments.root])
    if arguments.skills_command == "selection-validate":
        return validate_selection(request.raw, roots=[parse_skill_root(root) for root in arguments.root])
    if arguments.skills_command == "catalog":
        return catalog([parse_skill_root(root) for root in arguments.root], disabled_sources=set(arguments.disabled_source))
    return snapshot(parse_skill_root(arguments.root), arguments.source)
