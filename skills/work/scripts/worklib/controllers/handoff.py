from __future__ import annotations

import argparse

from ..workflows.handoff import run_handoff
from . import SubparserRegistry


def register_handoff_commands(commands: SubparserRegistry) -> None:
    handoff_parser = commands.add_parser("handoff")
    handoff_commands = handoff_parser.add_subparsers(
        dest="handoff_command", required=True
    )
    for command_name in ("validate", "render"):
        handoff_command = handoff_commands.add_parser(command_name)
        handoff_command.add_argument("--input-file", required=True)
    for command_name in ("verify-plan-to-task", "verify-task-to-execute"):
        verify = handoff_commands.add_parser(command_name, help="Verify incoming handoff identity against the selected current source.")
        verify.add_argument("--input-file", required=True)
        verify.add_argument("--user-config-root", required=True)
        verify.add_argument("--skill-root", action="append", default=[])
        if command_name == "verify-plan-to-task":
            verify.add_argument("--plan-path", required=True)
        else:
            verify.add_argument("--task-path", required=True)
            verify.add_argument("--task-id", required=True)
    for command_name in ("build-plan-to-task", "build-task-to-execute", "build-task-to-plan", "build-execute-to-task", "build-execute-to-plan"):
        build = handoff_commands.add_parser(command_name, help="Build a handoff from a validated formal artifact and semantic input.")
        build.add_argument("--input-file", required=True)
        build.add_argument("--user-config-root", required=True)
        build.add_argument("--skill-root", action="append", default=[])
        if command_name == "build-plan-to-task":
            build.add_argument("--plan-path", required=True)
        else:
            build.add_argument("--task-path", required=True)
            build.add_argument("--task-id", required=command_name != "build-task-to-plan")
            if command_name.startswith("build-execute-"):
                context = build.add_mutually_exclusive_group(required=True)
                context.add_argument("--attempt-id")
                context.add_argument("--preflight", action="store_true")
    for command_name in ("verify-task-to-plan", "verify-execute-to-plan", "verify-execute-to-task"):
        verify = handoff_commands.add_parser(command_name)
        verify.add_argument("--input-file", required=True)
        verify.add_argument("--plan-path", required=True)
        verify.add_argument("--task-path", required=True)
        verify.add_argument("--user-config-root", required=True)
        verify.add_argument("--skill-root", action="append", default=[])
        verify.add_argument("--task-id", required=command_name != "verify-task-to-plan")
        if command_name != "verify-task-to-plan":
            context = verify.add_mutually_exclusive_group(required=True)
            context.add_argument("--attempt-id")
            context.add_argument("--preflight", action="store_true")
