from __future__ import annotations

import argparse
from pathlib import Path

from ..orchestration.task import TaskRequestInput, execute_task_command
from . import SubparserRegistry


def _add_draft_source_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--requirement-id", required=True)
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--expected-revision", type=int, required=True)
    parser.add_argument("--plan-path", required=True)
    parser.add_argument("--user-config-root", required=True)
    parser.add_argument("--skill-root", action="append", default=[])
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--general-only", action="store_true")
    selection.add_argument("--instruction-path", action="append")
    parser.add_argument("--reference", action="append")


def register_task_commands(commands: SubparserRegistry) -> None:
    task_parser = commands.add_parser("task")
    task_commands = task_parser.add_subparsers(dest="task_command", required=True)

    for name in ("spec-prepare", "repair-prepare"):
        prepare = task_commands.add_parser(name, help="Prepare validated field replacements without publishing.")
        prepare.add_argument("--input-file", required=True)
        prepare.add_argument("--user-config-root", required=True)
        prepare.add_argument("--skill-root", action="append", default=[])
        prepare.add_argument("--output-file")
        if name != "repair-prepare":
            prepare.add_argument("--summary", action="store_true")

    for name in ("spec-validate", "spec-update", "spec-recover"):
        spec = task_commands.add_parser(name, help="Internal coordinated specification revision.")
        spec.add_argument("--input-file", required=True)
        spec.add_argument("--user-config-root", required=True)
        spec.add_argument("--skill-root", action="append", default=[])
        if name != "spec-validate":
            spec.add_argument("--approved-sha256", required=True)
        if name in {"spec-validate", "spec-update"}:
            spec.add_argument("--summary", action="store_true")

    for name in ("repair-validate", "repair", "repair-recover"):
        repair = task_commands.add_parser(name, help="Review or publish an explicitly decided TASK repair.")
        repair.add_argument("--input-file", required=True)
        repair.add_argument("--user-config-root", required=True)
        repair.add_argument("--skill-root", action="append", default=[])
        if name != "repair-validate":
            repair.add_argument("--approved-sha256", required=True)

    for name in ("spec-verify",):
        inspection = task_commands.add_parser(name, help="Inspect specification evidence without writes.")
        inspection.add_argument("--input-file", required=True)
        inspection.add_argument("--user-config-root", required=True)
        inspection.add_argument("--skill-root", action="append", default=[])

    migration_prepare = task_commands.add_parser("migration-prepare", help="Build cross-file migration candidates from semantic decisions.")
    migration_prepare.add_argument("--input-file", required=True)
    migration_prepare.add_argument("--user-config-root", required=True)
    migration_prepare.add_argument("--skill-root", action="append", default=[])
    migration_prepare.add_argument("--output-file")
    migration = task_commands.add_parser("migration-preview", help="Validate a Python-prepared cross-file migration without writing.")
    migration.add_argument("--input-file", required=True)
    migration.add_argument("--user-config-root", required=True)
    migration.add_argument("--skill-root", action="append", default=[])
    for name in ("migration-apply", "migration-recover"):
        publication = task_commands.add_parser(name, help="Publish or recover an approved cross-file migration.")
        publication.add_argument("--input-file", required=True)
        publication.add_argument("--user-config-root", required=True)
        publication.add_argument("--skill-root", action="append", default=[])
        publication.add_argument("--approved-sha256", required=True)
    reconciliation = task_commands.add_parser("reconciliation-preview", help="Review specification candidates derived from execution deviations.")
    reconciliation.add_argument("--input-file", required=True)
    reconciliation.add_argument("--user-config-root", required=True)
    reconciliation.add_argument("--skill-root", action="append", default=[])
    reconciliation_prepare = task_commands.add_parser("reconciliation-prepare", help="Build reconciliation candidates from semantic specification edits.")
    reconciliation_prepare.add_argument("--input-file", required=True)
    reconciliation_prepare.add_argument("--user-config-root", required=True)
    reconciliation_prepare.add_argument("--skill-root", action="append", default=[])
    reconciliation_prepare.add_argument("--output-file")
    reconciliation_apply = task_commands.add_parser("reconciliation-apply", help="Publish an approved specification reconciliation.")
    reconciliation_apply.add_argument("--input-file", required=True)
    reconciliation_apply.add_argument("--user-config-root", required=True)
    reconciliation_apply.add_argument("--skill-root", action="append", default=[])
    reconciliation_apply.add_argument("--approved-sha256", required=True)

    draft_init = task_commands.add_parser("draft-init", help="Save an initial planning index from a JSON request file.")
    draft_init.add_argument("--input-file", required=True)
    semantic = task_commands.add_parser("semantic-prepare", help="Derive TASK planning machine fields from semantic input.")
    semantic.add_argument("--input-file", required=True)
    semantic.add_argument("--requirement-id", required=True)
    semantic.add_argument("--plan-path", required=True)
    semantic.add_argument("--user-config-root", required=True)
    semantic.add_argument("--skill-root", action="append", default=[])
    semantic.add_argument("--expected-revision", type=int, default=0)
    draft_save = task_commands.add_parser("draft-save", help="Save one discussion from an index/draft JSON object.")
    draft_save.add_argument("--input-file", required=True)
    draft_save.add_argument("--expected-revision", type=int, required=True)
    draft_recover = task_commands.add_parser("draft-recover", help="Recover a fully prepared save using its original JSON request.")
    draft_recover.add_argument("--input-file", required=True)
    draft_recover.add_argument("--expected-revision", type=int, required=True)
    draft_read = task_commands.add_parser("draft-read", help="Read the committed index or one historical draft.")
    draft_read.add_argument("--requirement-id", required=True)
    draft_read.add_argument("--task-id")
    draft_status = task_commands.add_parser("draft-status", help="Inspect planning progress and the next action without writing.")
    draft_status.add_argument("--requirement-id", required=True)
    draft_status.add_argument("--task-id")
    draft_check = task_commands.add_parser("draft-check", help="Verify one TASK's saved source fingerprints without writing.")
    _add_draft_source_arguments(draft_check)
    for command_name in ("draft-save-request", "draft-recover-request"):
        draft_request = task_commands.add_parser(command_name, help="Save or recover one discussion with derived index and draft metadata.")
        _add_draft_source_arguments(draft_request)
        draft_request.add_argument("--input-file", required=True)
    for command_name in ("draft-list-update", "draft-list-recover"):
        draft_list = task_commands.add_parser(command_name)
        draft_list.add_argument("--input-file", required=True)
        draft_list.add_argument("--expected-revision", type=int, required=True)

    diagnose = task_commands.add_parser("diagnose", help="Diagnose an existing TASK without writing.")
    diagnose.add_argument("--path", required=True)
    diagnose.add_argument("--plan-path", required=True)
    diagnose.add_argument("--execution-dir", required=True)
    diagnose.add_argument("--user-config-root", required=True)
    diagnose.add_argument("--skill-root", action="append", default=[])

    task_validate = task_commands.add_parser("validate")
    for command_name in ("draft-source-update", "draft-source-recover"):
        source_update = task_commands.add_parser(command_name)
        source_update.add_argument("--input-file", required=True)
        source_update.add_argument("--requirement-id", required=True)
        source_update.add_argument("--expected-revision", type=int, required=True)
        source_update.add_argument("--plan-path", required=True)
        source_update.add_argument("--user-config-root", required=True)
        source_update.add_argument("--skill-root", action="append", default=[])
    for command_name in ("draft-assemble", "draft-create"):
        assembly = task_commands.add_parser(command_name)
        assembly.add_argument("--input-file", required=True)
        assembly.add_argument("--requirement-id", required=True)
        assembly.add_argument("--expected-revision", type=int, required=True)
        assembly.add_argument("--plan-path", required=True)
        assembly.add_argument("--user-config-root", required=True)
        assembly.add_argument("--skill-root", action="append", default=[])
        if command_name == "draft-create":
            assembly.add_argument("--approved-sha256", required=True)
    task_validate.add_argument("--user-config-root", required=True)
    task_validate.add_argument("--skill-root", action="append", default=[])
    task_source = task_validate.add_mutually_exclusive_group(required=True)
    task_source.add_argument("--path")
    task_source.add_argument("--input-file")
    task_validate.add_argument("--task-path")

    for command_name in ("create", "recover-create"):
        task_write = task_commands.add_parser(command_name)
        task_write.add_argument("--user-config-root", required=True)
        task_write.add_argument("--skill-root", action="append", default=[])
        task_write.add_argument("--input-file", required=True)
        task_write.add_argument("--plan-path", required=True)
        task_write.add_argument("--task-path", required=True)
        task_write.add_argument("--execution-dir", required=True)


def run_task(
    arguments: argparse.Namespace,
    project_root: Path,
    request: TaskRequestInput | None,
) -> dict[str, object]:
    return execute_task_command(arguments, project_root, request)
