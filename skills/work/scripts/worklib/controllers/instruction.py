from __future__ import annotations

import argparse
from pathlib import Path

from . import SubparserRegistry
from ..business_services.instruction import apply_instruction_migration, apply_source_refresh, apply_source_refresh_all, catalog, instruction_root, load, preview_instruction_migration, preview_source_refresh, preview_source_refresh_all, resolve, select, source_impact


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
    impact = subcommands.add_parser("impact")
    preview = subcommands.add_parser("refresh-preview")
    preview.add_argument("--requirement-id", required=True)
    apply = subcommands.add_parser("refresh-apply")
    apply.add_argument("--requirement-id", required=True)
    apply.add_argument("--approved-sha256", required=True)
    recover = subcommands.add_parser("refresh-recover")
    recover.add_argument("--requirement-id", required=True)
    recover.add_argument("--approved-sha256", required=True)
    subcommands.add_parser("refresh-preview-all")
    apply_all = subcommands.add_parser("refresh-apply-all")
    apply_all.add_argument("--approved-sha256", required=True)
    recover_all = subcommands.add_parser("refresh-recover-all")
    recover_all.add_argument("--approved-sha256", required=True)
    migration_preview = subcommands.add_parser("migration-preview")
    migration_preview.add_argument("--requirement-id", required=True)
    migration_apply = subcommands.add_parser("migration-apply")
    migration_apply.add_argument("--requirement-id", required=True)
    migration_apply.add_argument("--approved-sha256", required=True)


def run_instructions(arguments: argparse.Namespace, project_root: Path) -> dict[str, object]:
    skill_root = instruction_root()
    if arguments.instructions_command == "impact":
        return source_impact(project_root, skill_root)
    if arguments.instructions_command == "refresh-preview":
        return preview_source_refresh(project_root, skill_root, arguments.requirement_id)
    if arguments.instructions_command == "refresh-apply":
        return apply_source_refresh(project_root, skill_root, arguments.requirement_id, arguments.approved_sha256)
    if arguments.instructions_command == "refresh-recover":
        return apply_source_refresh(project_root, skill_root, arguments.requirement_id, arguments.approved_sha256, operation="recover")
    if arguments.instructions_command == "refresh-preview-all":
        return preview_source_refresh_all(project_root, skill_root)
    if arguments.instructions_command == "refresh-apply-all":
        return apply_source_refresh_all(project_root, skill_root, arguments.approved_sha256)
    if arguments.instructions_command == "refresh-recover-all":
        return apply_source_refresh_all(
            project_root, skill_root, arguments.approved_sha256, operation="recover"
        )
    if arguments.instructions_command == "migration-preview":
        return preview_instruction_migration(project_root, skill_root, arguments.requirement_id)
    if arguments.instructions_command == "migration-apply":
        return apply_instruction_migration(project_root, skill_root, arguments.requirement_id, arguments.approved_sha256)
    if arguments.instructions_command == "catalog":
        return catalog(skill_root, arguments.mode)
    if arguments.instructions_command == "load":
        return load(skill_root, arguments.mode, arguments.paths, arguments.reference)
    if arguments.instructions_command == "select":
        return select(skill_root, arguments.mode, arguments.paths, arguments.reference)
    result = resolve(skill_root, arguments.mode, arguments.paths)
    result["project_root"] = str(project_root)
    return result
