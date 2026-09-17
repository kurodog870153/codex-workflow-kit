from __future__ import annotations

from . import SubparserRegistry
from ..infrastructure.cli_io import FileInput
from ..services.invocation import parse_invocation


def register_invocation_commands(commands: SubparserRegistry) -> None:
    invocation = commands.add_parser("invocation")
    subcommands = invocation.add_subparsers(dest="invocation_command", required=True)
    parse = subcommands.add_parser(
        "parse", help="Parse an explicit Work invocation from a UTF-8 text file."
    )
    parse.add_argument("--input-file", required=True)


def run_invocation(request: FileInput) -> dict[str, object]:
    return parse_invocation(request.raw, source=request.source).to_canonical_dict()
