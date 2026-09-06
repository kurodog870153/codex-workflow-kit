from __future__ import annotations

from pathlib import Path


def installed_work_root() -> Path:
    """Return the root of the installed Work skill bundle."""
    return Path(__file__).resolve().parents[3]
