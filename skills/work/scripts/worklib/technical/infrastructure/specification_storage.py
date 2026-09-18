"""Low-level filesystem operations for Specification transactions."""

from __future__ import annotations

import os
from pathlib import Path

from .path_safety import resolve_project_relative_path
from ...models.common.errors import ExitCode, WorkError


def _error(code: str, message: str) -> WorkError:
    return WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message)


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
            raise WorkError(
                ExitCode.CONTRACT,
                "spec_update_link",
                "Specification storage cannot contain links.",
            )
    if resolved.is_file() and resolved.stat().st_nlink != 1:
        raise WorkError(
            ExitCode.CONTRACT,
            "spec_update_alias",
            "Specification storage cannot contain hard links.",
        )
    return resolved


def write_exclusive(path: Path, raw: bytes) -> None:
    """Create new storage and sync its bytes; preserve failures for recovery."""
    with path.open("xb") as output:
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())


def complete_write(path: Path, target: bytes) -> None:
    """Authorized recovery may append a missing suffix, never overwrite bytes."""
    if not path.exists():
        write_exclusive(path, target)
        return
    with path.open("r+b") as output:
        current = output.read()
        if not target.startswith(current):
            raise _error(
                "spec_update_partial_conflict",
                "Partial transaction bytes conflict with approval.",
            )
        if current != target:
            output.write(target[len(current):])
            output.flush()
            os.fsync(output.fileno())


def replace_checked(
    path: Path,
    expected: bytes,
    target: bytes,
    temporary: Path,
    *,
    recover: bool = False,
) -> None:
    """Publish prepared bytes only while the original bytes still match."""
    if recover:
        complete_write(temporary, target)
    elif temporary.exists():
        if temporary.read_bytes() != target:
            raise _error(
                "spec_update_temporary_changed",
                "Prepared specification bytes changed.",
            )
    else:
        write_exclusive(temporary, target)
    if path.read_bytes() != expected:
        raise _error(
            "spec_update_concurrent_change",
            "An artifact changed before publication.",
        )
    os.replace(temporary, path)
    if path.read_bytes() != target:
        raise _error(
            "spec_update_write_mismatch",
            "Published specification bytes differ from the approved candidate.",
        )


def replace_journal(path: Path, raw: bytes) -> None:
    """Atomically replace a transaction journal with exact approved bytes."""
    temporary = Path(str(path) + ".tmp")
    if temporary.exists():
        if temporary.read_bytes() != raw:
            raise _error(
                "spec_transaction_journal_temporary_changed",
                "Prepared journal bytes changed.",
            )
    else:
        write_exclusive(temporary, raw)
    os.replace(temporary, path)


def publish_snapshot(
    path: Path,
    before: bytes | None,
    after: bytes | None,
    temporary: Path,
) -> None:
    """Compare and publish one approved transaction snapshot."""
    current = path.read_bytes() if path.is_file() else None
    if current == after:
        return
    if current != before:
        raise _error(
            "spec_transaction_concurrent_change",
            "An artifact changed outside the transaction.",
        )
    if after is None:
        path.unlink()
    elif before is None:
        path.parent.mkdir(parents=True, exist_ok=True)
        write_exclusive(path, after)
    else:
        replace_checked(path, before, after, temporary, recover=True)


__all__ = [
    "complete_write", "publish_snapshot", "replace_checked", "replace_journal",
    "storage_path", "write_exclusive",
]
