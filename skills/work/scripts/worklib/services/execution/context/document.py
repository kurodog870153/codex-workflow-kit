"""Execution context document and path adapters."""

from pathlib import Path
from typing import Any

from ....technical.infrastructure.file_io import read_raw
from ....technical.infrastructure.json_contract import parse_json_contract
from ....technical.infrastructure.work_paths import resolve_project_relative_path


def read_contract(path: Path) -> tuple[bytes, dict[str, Any]]:
    raw = read_raw(path)
    return raw, parse_json_contract(raw, source=str(path))


__all__ = ["read_contract", "resolve_project_relative_path"]
