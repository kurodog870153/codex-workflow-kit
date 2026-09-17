"""Exclusive durable storage for command execution receipts."""
from __future__ import annotations

import os
from pathlib import Path


def write_command_receipt(path: Path, content: bytes) -> None:
    with path.open("xb") as stream:
        stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())
