"""Plan document and path operations."""

from pathlib import Path
from typing import Any

from ...technical.infrastructure.file_io import read_raw
from ...technical.infrastructure.json_contract import parse_json_contract, render_json_contract
from ...technical.infrastructure.work_paths import default_artifact_paths, resolve_project_relative_path, validate_artifact_paths
from ...technical.foundation.runtime import installed_work_root


def parse(raw: bytes, *, source: str) -> dict[str, Any]:
    return parse_json_contract(raw, source=source)


def render(contract: dict[str, Any]) -> bytes:
    return render_json_contract(contract)


def read(path: Path) -> bytes:
    return read_raw(path)


__all__ = ["default_artifact_paths", "installed_work_root", "parse", "read", "render", "resolve_project_relative_path", "validate_artifact_paths"]
