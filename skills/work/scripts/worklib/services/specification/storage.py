"""Specification storage access."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from ...technical.infrastructure.file_io import read_raw
from ...technical.infrastructure.work_paths import resolve_project_relative_path
from ...technical.infrastructure.specification_storage import storage_path


def write_prepared_output(path: str, value: dict[str, Any]) -> None:
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8") + b"\n"
    with Path(path).open("xb") as stream:
        stream.write(raw)


__all__ = [
    "read_raw",
    "resolve_project_relative_path",
    "storage_path",
    "write_prepared_output",
]
