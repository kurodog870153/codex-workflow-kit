from __future__ import annotations

import argparse

from . import SubparserRegistry


def register_contract_commands(commands: SubparserRegistry) -> None:
    parser = commands.add_parser("contract")
    subcommands = parser.add_subparsers(dest="contract_command", required=True)
    subcommands.add_parser("list")
    describe = subcommands.add_parser("describe")
    describe.add_argument("contract_id")
    scaffold = subcommands.add_parser("scaffold")
    scaffold.add_argument("contract_id")


def run_contract(arguments: argparse.Namespace) -> dict[str, object]:
    from ..business_services.contract import describe_contract, list_contracts, scaffold_contract

    if arguments.contract_command == "list":
        return list_contracts()
    if arguments.contract_command == "scaffold":
        return scaffold_contract(arguments.contract_id)
    return describe_contract(arguments.contract_id)
