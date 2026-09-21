from __future__ import annotations

from pathlib import Path
import sys


MINIMUM_PYTHON = (3, 14)


def supported_python(version: tuple[int, ...] | None = None) -> bool:
    current = version if version is not None else sys.version_info
    return current >= MINIMUM_PYTHON


def installed_work_root() -> Path:
    """Return the root of the installed Work skill bundle."""
    return Path(__file__).resolve().parents[4]
