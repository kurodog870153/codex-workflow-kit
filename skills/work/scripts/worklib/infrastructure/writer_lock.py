"""Cross-process writer mutexes for Work artifact publication."""
from __future__ import annotations

import os
from contextlib import contextmanager
from pathlib import Path
from typing import BinaryIO, Iterator

from ..models.common.errors import ExitCode, WorkError
from ..foundation.spec_update import storage_path


@contextmanager
def _writer_lock(stream: BinaryIO) -> Iterator[None]:
    try:
        if os.name == "nt":
            import msvcrt

            stream.seek(0)
            msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError as error:
        raise WorkError(
            ExitCode.LOCK_CONFLICT,
            "work_state_writer_busy",
            "Another Work command is updating this requirement.",
        ) from error
    try:
        yield
    finally:
        if os.name == "nt":
            stream.seek(0)
            msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def require_idle_writer(root: Path, execution_dir: str) -> None:
    """Probe an existing OS mutex without creating files or changing bytes."""
    path = storage_path(root, execution_dir + "/.work-state-writer.lock")
    if path.exists():
        with path.open("rb") as stream, _writer_lock(stream):
            pass


@contextmanager
def state_writer(root: Path, execution_dir: str) -> Iterator[None]:
    """Serialize cooperating Work CLI writers; an OS lock dies with its process."""
    path = storage_path(root, execution_dir + "/.work-state-writer.lock")
    with path.open("a+b") as stream, _writer_lock(stream):
        yield
