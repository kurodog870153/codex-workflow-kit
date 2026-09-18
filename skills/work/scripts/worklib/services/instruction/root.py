"""Resolve the installed Work instruction root."""

from pathlib import Path

from ...technical.foundation.runtime import installed_work_root


def instruction_root() -> Path:
    return installed_work_root()


__all__ = ["instruction_root"]
