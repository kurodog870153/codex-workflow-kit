from __future__ import annotations

import argparse
from pathlib import Path

from ..orchestration.workflow import workflow_state
from . import SubparserRegistry


def register_workflow_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("workflow")
    operations = parser.add_subparsers(dest="workflow_command", required=True)
    for name in ("status", "next"):
        command = operations.add_parser(name, help="Inspect deterministic workflow continuation without writing.")
        command.add_argument("--requirement-id", required=True)
        command.add_argument("--plan-path")
        command.add_argument("--user-config-root", required=True)
        command.add_argument("--skill-root", action="append", default=[])


def run_workflow(arguments: argparse.Namespace, project_root: Path) -> dict[str, object]:
    return workflow_state(project_root, arguments.requirement_id,
                          plan_path=arguments.plan_path,
                          user_config_root=arguments.user_config_root,
                          skill_roots=arguments.skill_root)
