from __future__ import annotations

import os
from collections.abc import Callable
from pathlib import Path

from ..foundation.errors import ExitCode, WorkError
from ..foundation.spec_update import storage_path
from .writer_lock import state_writer


def read_progress_bytes(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "progress_read_failed",
            "The progress file could not be read.",
            {"path": str(path)},
        ) from error


def write_progress_bytes(path: Path, raw: bytes) -> None:
    with path.open("xb") as output:
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())
    if read_progress_bytes(path) != raw:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "progress_write_mismatch",
            "Saved progress differs from the reviewed bytes.",
        )


def replace_progress_file(source: Path, destination: Path) -> None:
    os.replace(source, destination)


def publish_progress_revision(
    project_root: Path,
    *,
    directory: str,
    history: str,
    current_path: str,
    prepare_raw: Callable[[], bytes],
) -> bytes:
    try:
        storage_path(project_root, directory).mkdir(parents=True, exist_ok=True)
        # This mutex lives only in progress storage, independently of Execute locks.
        with state_writer(project_root, directory):
            raw = prepare_raw()
            storage_path(project_root, history).mkdir(parents=True, exist_ok=False)
            write_progress_bytes(storage_path(project_root, history + "/progress.json"), raw)
            pending = storage_path(project_root, history + "/progress.pending")
            write_progress_bytes(pending, raw)
            # This replacement is the commit point. Before it, readers still see
            # the previous complete snapshot and its immutable history.
            replace_progress_file(pending, storage_path(project_root, current_path))
            return raw
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "progress_save_interrupted",
            "Saving was interrupted. Preserve all files and read the last committed progress before deciding how to continue.",
            {"path": current_path, "history": history},
        ) from error
