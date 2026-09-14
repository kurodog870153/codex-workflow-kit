from __future__ import annotations

from ..contracts.delegation import ROLES, validate_delegation
from ..foundation.markdown import parse_json_contract
from ..foundation.runtime import installed_work_root
from . import SubparserRegistry


def register_delegation_commands(commands: SubparserRegistry):
    delegation = commands.add_parser("delegation")
    subcommands = delegation.add_subparsers(dest="delegation_command", required=True)
    validate = subcommands.add_parser("validate", help="Check an internal role envelope without granting authority.")
    validate.add_argument("--input-file", required=True)
    validate.add_argument("--role", choices=ROLES, required=True)
    validate.add_argument("--sender", choices=("parent", "task-coordinator"), required=True)


def run_delegation(arguments, project_root, request):
    return validate_delegation(parse_json_contract(request.raw, source=request.source),
        role=arguments.role, sender=arguments.sender, project_root=project_root, skill_root=installed_work_root())
