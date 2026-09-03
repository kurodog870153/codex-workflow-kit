from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Sequence, TextIO

from .contracts.attempt import (
    render_attempt_json_contract,
    validate_attempt_file,
    validate_attempt_json_contract,
)
from .contracts.correction import (
    render_correction_json_contract,
    validate_correction_file,
    validate_correction_json_contract,
)
from .foundation.errors import ExitCode, WorkError
from .execution.attempt_close import close_attempt
from .execution.attempt_start import recover_attempt_start, start_attempt
from .execution.command_correction import record_command_correction
from .execution.correction import create_correction
from .execution.preflight import execute_preflight
from .execution.record_begin import begin_record
from .execution.record_finish import finish_record
from .execution.recovery import recover_execution
from .execution.worktree import inspect_execute_worktree
from .foundation.fingerprint import fingerprint_file
from .contracts.handoff import (
    render_handoff_json_contract,
    validate_handoff_json_contract,
)
from .foundation.hierarchy import build_hierarchy
from .hierarchy.selection import (
    build_hierarchy_selection_json,
    validate_hierarchy_selection_json,
)
from .instructions.selection import build_instruction_selection
from .instructions.sources import load_instruction_sources
from .instructions.catalog import (
    build_cross_mode_instruction_catalog,
    build_instruction_catalog,
    resolve_instruction_hierarchy,
)
from .foundation.jsonio import write_json
from .foundation.paths import default_artifact_paths, resolve_project_relative_path, resolve_root
from .contracts.plan import (
    create_plan_file,
    validate_plan_file,
    validate_plan_json_contract,
)
from .skills.catalog import (
    build_skill_catalog,
    parse_skill_root,
    snapshot_catalog_skill,
)
from .skills.selection import validate_skill_selection_json
from .contracts.task import validate_task_file, validate_task_json_contract
from .contracts.task import create_task_artifacts, recover_task_create


class WorkArgumentParser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise WorkError(
            ExitCode.CLI_USAGE,
            "cli_usage_error",
            "The CLI arguments are invalid.",
            {"reason": message},
        )


def build_parser() -> WorkArgumentParser:
    parser = WorkArgumentParser(prog="work.py")
    parser.add_argument("--project-root", required=True)
    commands = parser.add_subparsers(dest="command", required=True)

    paths_parser = commands.add_parser("paths")
    paths_commands = paths_parser.add_subparsers(dest="paths_command", required=True)
    paths_resolve = paths_commands.add_parser("resolve")
    paths_resolve.add_argument("--requirement-id", required=True)

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
        hierarchy_selection.add_argument("--stdin", action="store_true", required=True)

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
    instructions_load = instructions_commands.add_parser("load")
    instructions_load.add_argument(
        "--mode", choices=("plan", "task", "execute"), required=True
    )
    instructions_load.add_argument("--reference", action="append", default=[])
    instructions_load.add_argument("paths", nargs="*")
    instructions_select = instructions_commands.add_parser("select")
    instructions_select.add_argument(
        "--mode", choices=("plan", "task", "execute"), required=True
    )
    instructions_select.add_argument("--reference", action="append", default=[])
    instructions_select.add_argument("paths", nargs="*")

    fingerprint_parser = commands.add_parser("fingerprint")
    fingerprint_commands = fingerprint_parser.add_subparsers(
        dest="fingerprint_command", required=True
    )
    fingerprint_text = fingerprint_commands.add_parser("text")
    fingerprint_text.add_argument("--path", required=True)

    skills_parser = commands.add_parser("skills")
    skills_commands = skills_parser.add_subparsers(dest="skills_command", required=True)
    skills_catalog = skills_commands.add_parser("catalog")
    skills_catalog.add_argument("--root", action="append", required=True)
    skills_catalog.add_argument("--disabled-source", action="append", default=[])
    skills_snapshot = skills_commands.add_parser("snapshot")
    skills_snapshot.add_argument("--root", required=True)
    skills_snapshot.add_argument("--source", required=True)
    skills_selection_validate = skills_commands.add_parser("selection-validate")
    skills_selection_validate.add_argument("--root", action="append", required=True)
    skills_selection_validate.add_argument("--stdin", action="store_true", required=True)

    handoff_parser = commands.add_parser("handoff")
    handoff_commands = handoff_parser.add_subparsers(
        dest="handoff_command", required=True
    )
    for command_name in ("validate", "render"):
        handoff_command = handoff_commands.add_parser(command_name)
        handoff_command.add_argument("--stdin", action="store_true", required=True)

    attempt_parser = commands.add_parser("attempt")
    attempt_commands = attempt_parser.add_subparsers(
        dest="attempt_command", required=True
    )
    attempt_validate = attempt_commands.add_parser("validate")
    attempt_source = attempt_validate.add_mutually_exclusive_group(required=True)
    attempt_source.add_argument("--path")
    attempt_source.add_argument("--stdin", action="store_true")
    attempt_render = attempt_commands.add_parser("render")
    attempt_render.add_argument("--stdin", action="store_true", required=True)

    correction_parser = commands.add_parser("correction")
    correction_commands = correction_parser.add_subparsers(
        dest="correction_command", required=True
    )
    correction_validate = correction_commands.add_parser("validate")
    correction_source = correction_validate.add_mutually_exclusive_group(required=True)
    correction_source.add_argument("--path")
    correction_source.add_argument("--stdin", action="store_true")
    correction_render = correction_commands.add_parser("render")
    correction_render.add_argument("--stdin", action="store_true", required=True)

    plan_parser = commands.add_parser("plan")
    plan_commands = plan_parser.add_subparsers(dest="plan_command", required=True)
    plan_validate = plan_commands.add_parser("validate")
    plan_validate.add_argument("--user-config-root", required=True)
    plan_validate.add_argument("--skill-root", action="append", default=[])
    plan_source = plan_validate.add_mutually_exclusive_group(required=True)
    plan_source.add_argument("--path")
    plan_source.add_argument("--stdin", action="store_true")
    plan_validate.add_argument("--plan-path")
    plan_create = plan_commands.add_parser("create")
    plan_create.add_argument("--user-config-root", required=True)
    plan_create.add_argument("--skill-root", action="append", default=[])
    plan_create.add_argument("--stdin", action="store_true", required=True)
    plan_create.add_argument("--plan-path", required=True)

    task_parser = commands.add_parser("task")
    task_commands = task_parser.add_subparsers(dest="task_command", required=True)
    task_validate = task_commands.add_parser("validate")
    task_validate.add_argument("--user-config-root", required=True)
    task_validate.add_argument("--skill-root", action="append", default=[])
    task_source = task_validate.add_mutually_exclusive_group(required=True)
    task_source.add_argument("--path")
    task_source.add_argument("--stdin", action="store_true")
    task_validate.add_argument("--task-path")
    for command_name in ("create", "recover-create"):
        task_write = task_commands.add_parser(command_name)
        task_write.add_argument("--user-config-root", required=True)
        task_write.add_argument("--skill-root", action="append", default=[])
        task_write.add_argument("--stdin", action="store_true", required=True)
        task_write.add_argument("--plan-path", required=True)
        task_write.add_argument("--task-path", required=True)
        task_write.add_argument("--execution-dir", required=True)

    execute_parser = commands.add_parser("execute")
    execute_commands = execute_parser.add_subparsers(
        dest="execute_command", required=True
    )
    for command_name in ("preflight", "worktree"):
        execute_command = execute_commands.add_parser(command_name)
        execute_command.add_argument("--user-config-root", required=True)
        execute_command.add_argument("--task-path", required=True)
        execute_command.add_argument("--execution-dir", required=True)
        execute_command.add_argument("--task-id", required=True)
        execute_command.add_argument("--skill-root", action="append", default=[])
        execute_command.add_argument(
            "--confirmed-input", action="append", default=[]
        )
    for command_name in ("attempt-start", "recover-attempt-start"):
        execute_command = execute_commands.add_parser(command_name)
        execute_command.add_argument("--user-config-root", required=True)
        execute_command.add_argument("--task-path", required=True)
        execute_command.add_argument("--execution-dir", required=True)
        execute_command.add_argument("--task-id", required=True)
        execute_command.add_argument("--skill-root", action="append", default=[])
        execute_command.add_argument(
            "--confirmed-input", action="append", default=[]
        )
        execute_command.add_argument("--stdin", action="store_true", required=True)
    record_begin = execute_commands.add_parser("record-begin")
    record_begin.add_argument("--user-config-root", required=True)
    record_begin.add_argument("--task-path", required=True)
    record_begin.add_argument("--execution-dir", required=True)
    record_begin.add_argument("--task-id", required=True)
    record_begin.add_argument("--skill-root", action="append", default=[])
    record_begin.add_argument("--record-id", required=True)
    command_correction = execute_commands.add_parser("command-correction")
    command_correction.add_argument("--user-config-root", required=True)
    command_correction.add_argument("--task-path", required=True)
    command_correction.add_argument("--execution-dir", required=True)
    command_correction.add_argument("--task-id", required=True)
    command_correction.add_argument("--skill-root", action="append", default=[])
    command_correction.add_argument("--stdin", action="store_true", required=True)
    record_finish = execute_commands.add_parser("record-finish")
    record_finish.add_argument("--user-config-root", required=True)
    record_finish.add_argument("--task-path", required=True)
    record_finish.add_argument("--execution-dir", required=True)
    record_finish.add_argument("--task-id", required=True)
    record_finish.add_argument("--skill-root", action="append", default=[])
    record_finish.add_argument("--stdin", action="store_true", required=True)
    attempt_close = execute_commands.add_parser("attempt-close")
    attempt_close.add_argument("--user-config-root", required=True)
    attempt_close.add_argument("--task-path", required=True)
    attempt_close.add_argument("--execution-dir", required=True)
    attempt_close.add_argument("--task-id", required=True)
    attempt_close.add_argument("--skill-root", action="append", default=[])
    attempt_close.add_argument("--stdin", action="store_true", required=True)
    correction_create = execute_commands.add_parser("correction-create")
    correction_create.add_argument("--user-config-root", required=True)
    correction_create.add_argument("--task-path", required=True)
    correction_create.add_argument("--execution-dir", required=True)
    correction_create.add_argument("--task-id", required=True)
    correction_create.add_argument("--skill-root", action="append", default=[])
    correction_create.add_argument("--stdin", action="store_true", required=True)
    recovery = execute_commands.add_parser("recover")
    recovery.add_argument("--user-config-root", required=True)
    recovery.add_argument("--task-path", required=True)
    recovery.add_argument("--execution-dir", required=True)
    recovery.add_argument("--task-id", required=True)
    recovery.add_argument("--skill-root", action="append", default=[])
    recovery.add_argument("--stdin", action="store_true", required=True)
    return parser


def _run(
    arguments: argparse.Namespace,
    project_root: Path,
    input_stream: TextIO,
) -> dict[str, object]:
    if arguments.command == "paths":
        return {
            "schema": "work-paths/v1",
            "project_root": str(project_root),
            "requirement_id": arguments.requirement_id,
            "paths": default_artifact_paths(project_root, arguments.requirement_id),
        }

    if arguments.command == "hierarchy":
        if arguments.hierarchy_command == "resolve":
            result = build_hierarchy(arguments.work_directory, arguments.paths).as_dict()
            result["project_root"] = str(project_root)
            return result
        skill_root = Path(__file__).resolve().parents[2]
        operation = (
            build_hierarchy_selection_json
            if arguments.hierarchy_command == "selection-build"
            else validate_hierarchy_selection_json
        )
        return operation(
            input_stream.read().encode("utf-8"),
            skill_root=skill_root,
        )

    if arguments.command == "instructions":
        skill_root = Path(__file__).resolve().parents[2]
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

    if arguments.command == "fingerprint":
        normalized, path = resolve_project_relative_path(
            project_root,
            arguments.path,
            field="path",
        )
        result: dict[str, object] = {
            "schema": "work-fingerprint/v1",
            "path": normalized,
        }
        result.update(fingerprint_file(path))
        return result

    if arguments.command == "skills":
        if arguments.skills_command == "selection-validate":
            return validate_skill_selection_json(
                input_stream.read().encode("utf-8"),
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

    if arguments.command == "handoff":
        operation = (
            validate_handoff_json_contract
            if arguments.handoff_command == "validate"
            else render_handoff_json_contract
        )
        return operation(
            input_stream.read().encode("utf-8"),
            source="stdin",
            project_root=project_root,
        )

    if arguments.command == "attempt":
        if arguments.attempt_command == "render":
            return render_attempt_json_contract(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
            )
        if arguments.stdin:
            return validate_attempt_json_contract(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
            )
        return validate_attempt_file(project_root, arguments.path)

    if arguments.command == "correction":
        if arguments.correction_command == "render":
            return render_correction_json_contract(
                input_stream.read().encode("utf-8"), source="stdin"
            )
        if arguments.stdin:
            return validate_correction_json_contract(
                input_stream.read().encode("utf-8"), source="stdin"
            )
        return validate_correction_file(project_root, arguments.path)

    if arguments.command == "plan":
        skill_roots = [parse_skill_root(root) for root in arguments.skill_root]
        if arguments.plan_command == "create":
            raw = input_stream.read().encode("utf-8")
            return create_plan_file(
                raw,
                source="stdin",
                raw_plan_path=arguments.plan_path,
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                skill_roots=skill_roots,
            )
        if arguments.stdin:
            if not arguments.plan_path:
                raise WorkError(
                    ExitCode.CLI_USAGE,
                    "plan_path_required",
                    "--plan-path is required with --stdin.",
                )
            raw = input_stream.read().encode("utf-8")
            return validate_plan_json_contract(
                raw,
                source="stdin",
                actual_plan_path=arguments.plan_path,
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                skill_roots=skill_roots,
            )
        if arguments.plan_path:
            raise WorkError(
                ExitCode.CLI_USAGE,
                "unexpected_plan_path",
                "--plan-path is only valid with --stdin.",
            )
        return validate_plan_file(
            project_root,
            arguments.user_config_root,
            arguments.path,
            skill_roots=skill_roots,
        )

    if arguments.command == "task":
        skill_roots = [parse_skill_root(root) for root in arguments.skill_root]
        if arguments.task_command in {"create", "recover-create"}:
            operation = (
                create_task_artifacts
                if arguments.task_command == "create"
                else recover_task_create
            )
            return operation(
                input_stream.read().encode("utf-8"),
                source="stdin",
                raw_plan_path=arguments.plan_path,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                skill_roots=skill_roots,
            )
        if arguments.stdin:
            if not arguments.task_path:
                raise WorkError(
                    ExitCode.CLI_USAGE,
                    "task_path_required",
                    "--task-path is required with --stdin.",
                )
            return validate_task_json_contract(
                input_stream.read().encode("utf-8"),
                source="stdin",
                actual_task_path=arguments.task_path,
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                skill_roots=skill_roots,
            )
        if arguments.task_path:
            raise WorkError(
                ExitCode.CLI_USAGE,
                "unexpected_task_path",
                "--task-path is only valid with --stdin.",
            )
        return validate_task_file(
            project_root,
            arguments.user_config_root,
            arguments.path,
            skill_roots=skill_roots,
        )

    if arguments.command == "execute":
        if arguments.execute_command == "record-begin":
            return begin_record(
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                task_id=arguments.task_id,
                base_record_id=arguments.record_id,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            )
        if arguments.execute_command == "command-correction":
            return record_command_correction(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                task_id=arguments.task_id,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            )
        if arguments.execute_command == "record-finish":
            return finish_record(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                task_id=arguments.task_id,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            )
        if arguments.execute_command == "attempt-close":
            return close_attempt(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                task_id=arguments.task_id,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            )
        if arguments.execute_command == "correction-create":
            return create_correction(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                task_id=arguments.task_id,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            )
        if arguments.execute_command == "recover":
            return recover_execution(
                input_stream.read().encode("utf-8"),
                source="stdin",
                project_root=project_root,
                user_config_root=arguments.user_config_root,
                raw_task_path=arguments.task_path,
                raw_execution_dir=arguments.execution_dir,
                task_id=arguments.task_id,
                skill_roots=[parse_skill_root(root) for root in arguments.skill_root],
            )
        common = {
            "project_root": project_root,
            "user_config_root": arguments.user_config_root,
            "raw_task_path": arguments.task_path,
            "raw_execution_dir": arguments.execution_dir,
            "task_id": arguments.task_id,
            "confirmed_inputs": arguments.confirmed_input,
            "skill_roots": [parse_skill_root(root) for root in arguments.skill_root],
        }
        if arguments.execute_command == "preflight":
            return execute_preflight(**common)
        if arguments.execute_command == "worktree":
            return inspect_execute_worktree(**common)
        operation = (
            start_attempt
            if arguments.execute_command == "attempt-start"
            else recover_attempt_start
        )
        return operation(
            input_stream.read().encode("utf-8"),
            source="stdin",
            **common,
        )

    raise WorkError(
        ExitCode.INTERNAL_ERROR,
        "unreachable_command",
        "The parsed command could not be dispatched.",
    )


def main(
    argv: Sequence[str] | None = None,
    *,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
    stdin: TextIO | None = None,
) -> int:
    output = stdout or sys.stdout
    error_output = stderr or sys.stderr
    input_stream = stdin or sys.stdin
    try:
        arguments = build_parser().parse_args(argv)
        project_root = resolve_root(arguments.project_root, label="project root")
        result = _run(arguments, project_root, input_stream)
        preserve_order = (
            arguments.command == "attempt"
            and arguments.attempt_command == "render"
        ) or (
            arguments.command == "correction"
            and arguments.correction_command == "render"
        )
        write_json(output, result, sort_keys=not preserve_order)
        return int(ExitCode.SUCCESS)
    except WorkError as error:
        write_json(error_output, error.as_dict())
        return int(error.exit_code)
    except Exception:
        error = WorkError(
            ExitCode.INTERNAL_ERROR,
            "internal_error",
            "An unexpected internal error occurred.",
        )
        write_json(error_output, error.as_dict())
        return int(error.exit_code)
