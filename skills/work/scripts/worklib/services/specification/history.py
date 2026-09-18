"""Specification execution-history fingerprinting."""

from __future__ import annotations

import os
from pathlib import Path

from ...technical.foundation.fingerprint import raw_sha256
from ...technical.infrastructure.specification_storage import storage_path
from ...models.common.errors import ExitCode, WorkError


def execution_history_fingerprints(root: Path, execution: str) -> dict[str, str]:
    result: dict[str, str] = {}
    directory = storage_path(root, execution)
    for task_dir in directory.glob("TASK-*"):
        safe = storage_path(root, task_dir.relative_to(root).as_posix())
        if not safe.is_dir():
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "spec_update_history_layout",
                "An execution TASK entry must be a directory.",
            )
        for current, directories, files in os.walk(safe, followlinks=False):
            for name in directories + files:
                path = storage_path(root, (Path(current) / name).relative_to(root).as_posix())
                if path.is_file():
                    result[path.relative_to(root).as_posix()] = raw_sha256(path.read_bytes())
    return result


__all__ = ["execution_history_fingerprints"]
