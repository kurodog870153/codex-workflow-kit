"""Correction CLI use cases."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Protocol

from ...services.attempt.validation import validate_attempt_file
from ...services.correction.document import (
    render_correction_json_contract,
    validate_correction_file,
    validate_correction_json_contract,
)


class RequestInput(Protocol):
    raw: bytes
    source: str


def handle_correction_request(
    arguments: argparse.Namespace,
    project_root: Path,
    request: RequestInput | None,
) -> dict[str, object]:
    if arguments.correction_command == "render":
        return render_correction_json_contract(request.raw, source=request.source)
    if arguments.input_file:
        return validate_correction_json_contract(request.raw, source=request.source)
    return validate_correction_file(
        project_root,
        arguments.path,
        validate_attempt_file=validate_attempt_file,
    )


__all__ = ["handle_correction_request"]
