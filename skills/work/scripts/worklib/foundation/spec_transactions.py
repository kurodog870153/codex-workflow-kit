"""Shared file primitives for specification and TASK repair transactions.

Callers own path validation, writer locks, approval and transaction ordering.
Recovery only appends matching bytes. Existing specification error codes remain
part of the shared behavior; these primitives never authorize recovery themselves.
"""

from __future__ import annotations

import os
from pathlib import Path

from .errors import ExitCode, WorkError


def _error(code: str, message: str) -> WorkError:
    return WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message)


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
            raise _error("spec_update_partial_conflict", "Partial transaction bytes conflict with approval.")
        if current != target:
            output.write(target[len(current):])
            output.flush()
            os.fsync(output.fileno())


def replace_checked(
    path: Path, expected: bytes, target: bytes, temporary: Path, *, recover: bool = False,
) -> None:
    """Publish prepared bytes only while the original bytes still match."""
    if recover:
        complete_write(temporary, target)
    elif temporary.exists():
        if temporary.read_bytes() != target:
            raise _error("spec_update_temporary_changed", "Prepared specification bytes changed.")
    else:
        write_exclusive(temporary, target)
    if path.read_bytes() != expected:
        raise _error("spec_update_concurrent_change", "An artifact changed before publication.")
    os.replace(temporary, path)
    if path.read_bytes() != target:
        raise _error("spec_update_write_mismatch", "Published specification bytes differ from the approved candidate.")
