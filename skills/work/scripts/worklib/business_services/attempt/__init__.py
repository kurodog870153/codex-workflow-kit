"""Attempt CLI use cases."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Protocol

from ...services.attempt.validation import (
    render_attempt_json_contract,
    validate_attempt_file,
    validate_attempt_json_contract,
)


class RequestInput(Protocol):
    raw: bytes
    source: str


def handle_attempt_request(
    arguments: argparse.Namespace,
    project_root: Path,
    request: RequestInput | None,
) -> dict[str, object]:
    if arguments.attempt_command == "render":
        return render_attempt_json_contract(
            request.raw,
            source=request.source,
            project_root=project_root,
        )
    if arguments.input_file:
        return validate_attempt_json_contract(
            request.raw,
            source=request.source,
            project_root=project_root,
        )
    return validate_attempt_file(project_root, arguments.path)


__all__ = ["handle_attempt_request"]
