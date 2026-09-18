from __future__ import annotations

import argparse

from . import SubparserRegistry
from ..contracts.registry import registry


def register_contract_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("contract")
    subcommands = parser.add_subparsers(dest="contract_command", required=True)
    subcommands.add_parser("list")
    describe = subcommands.add_parser("describe")
    describe.add_argument("contract_id")
    scaffold = subcommands.add_parser("scaffold")
    scaffold.add_argument("contract_id")


def run_contract(arguments: argparse.Namespace) -> dict[str, object]:
    if arguments.contract_command == "list":
        return registry.catalog().to_canonical_dict()
    if arguments.contract_command == "scaffold":
        return registry.scaffold(arguments.contract_id).to_canonical_dict()
    return registry.describe(arguments.contract_id).to_canonical_dict()
