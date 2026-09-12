"""Guards shared by specification updates and Execute entry points."""

from __future__ import annotations

import hashlib
import os
from contextlib import contextmanager
from pathlib import Path

from .errors import ExitCode, WorkError
from .paths import resolve_project_relative_path


def storage_path(root: Path, relative: str) -> Path:
    """Require ordinary storage, without links, junctions or hard-link aliases."""
    _, resolved = resolve_project_relative_path(root, relative, field="spec_update")
    candidate = root
    for part in relative.split("/"):
        candidate = candidate / part
        if candidate.is_symlink() or (
            candidate.exists()
            and getattr(candidate.lstat(), "st_file_attributes", 0) & 0x400
        ):
            raise WorkError(ExitCode.CONTRACT, "spec_update_link", "Specification storage cannot contain links.")
    if resolved.is_file() and resolved.stat().st_nlink != 1:
        raise WorkError(ExitCode.CONTRACT, "spec_update_alias", "Specification storage cannot contain hard links.")
    return resolved


def require_no_spec_update(root: Path, execution_dir: str, *, ignored_record: str | None = None) -> None:
    directory = storage_path(root, execution_dir)
    records = sorted([*directory.glob(".work-spec-update-*.json"), *directory.glob(".work-task-repair-*.json")])
    for record in records:
        if record.relative_to(root).as_posix() == ignored_record:
            continue
        record = storage_path(root, record.relative_to(root).as_posix())
        done = storage_path(root, record.relative_to(root).as_posix() + ".done")
        expected = hashlib.sha256(record.read_bytes()).hexdigest().encode("ascii") + b"\n"
        if not done.is_file() or done.read_bytes() != expected:
            raise WorkError(
                ExitCode.LOCK_CONFLICT, "spec_update_pending",
                "An incomplete specification update requires separately authorized recovery.",
                {"recovery_required": True, "record": record.relative_to(root).as_posix()},
            )

@contextmanager
def _writer_lock(stream):
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
            ExitCode.LOCK_CONFLICT, "work_state_writer_busy",
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


def require_idle_writer(root: Path, execution_dir: str):
    """Probe an existing OS mutex without creating files or changing bytes."""
    path = storage_path(root, execution_dir + "/.work-state-writer.lock")
    if path.exists():
        with path.open("rb") as stream, _writer_lock(stream):
            pass


@contextmanager
def state_writer(root: Path, execution_dir: str):
    """Serialize cooperating Work CLI writers; an OS lock dies with its process."""
    path = storage_path(root, execution_dir + "/.work-state-writer.lock")
    with path.open("a+b") as stream, _writer_lock(stream):
        yield
