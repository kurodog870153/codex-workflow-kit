from __future__ import annotations

import argparse

from ..skills.catalog import (
    build_skill_catalog,
    parse_skill_root,
    snapshot_catalog_skill,
)
from ..skills.selection import validate_skill_selection_json
from ..foundation.cli_io import FileInput
from . import SubparserRegistry


def register_skill_commands(commands: SubparserRegistry) -> None:
    skills_parser = commands.add_parser("skills")
    skills_commands = skills_parser.add_subparsers(
        dest="skills_command", required=True
    )

    skills_catalog = skills_commands.add_parser("catalog")
    skills_catalog.add_argument("--root", action="append", required=True)
    skills_catalog.add_argument("--disabled-source", action="append", default=[])

    skills_snapshot = skills_commands.add_parser("snapshot")
    skills_snapshot.add_argument("--root", required=True)
    skills_snapshot.add_argument("--source", required=True)

    skills_selection_validate = skills_commands.add_parser("selection-validate")
    skills_selection_validate.add_argument("--root", action="append", required=True)
    skills_selection_validate.add_argument(
        "--input-file", required=True
    )


def run_skills(
    arguments: argparse.Namespace,
    request: FileInput | None,
) -> dict[str, object]:
    if arguments.skills_command == "selection-validate":
        return validate_skill_selection_json(
            request.raw,
            roots=[parse_skill_root(root) for root in arguments.root],
        )
    if arguments.skills_command == "catalog":
        return build_skill_catalog(
            [parse_skill_root(root) for root in arguments.root],
            disabled_sources=set(arguments.disabled_source),
        )
    return snapshot_catalog_skill(
        parse_skill_root(arguments.root),
        arguments.source,
    )
