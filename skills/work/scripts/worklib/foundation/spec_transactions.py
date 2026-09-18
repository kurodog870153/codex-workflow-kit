"""Shared file primitives for specification and TASK repair transactions.

Callers own path validation, writer locks, approval and transaction ordering.
Recovery only appends matching bytes. Existing specification error codes remain
part of the shared behavior; these primitives never authorize recovery themselves.
"""

from __future__ import annotations

import os
from pathlib import Path

from ..contracts.spec_transaction import render_spec_transaction, validate_spec_transaction
from .fingerprint import read_raw, raw_sha256
from .spec_update import completion_marker_matches, storage_path
from ..models.common.errors import ExitCode, WorkError


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


def write_journal(path: Path, contract: object) -> bytes:
    raw = render_spec_transaction(contract)
    write_exclusive(path, raw)
    return raw


def _snapshot_bytes(row: dict[str, object], side: str) -> bytes | None:
    import base64
    snapshot = row.get(side)
    return None if snapshot is None else base64.b64decode(snapshot["base64"])  # type: ignore[index]


def _replace_journal(path: Path, contract: dict[str, object]) -> bytes:
    raw = render_spec_transaction(contract)
    temporary = Path(str(path) + ".tmp")
    if temporary.exists():
        if temporary.read_bytes() != raw:
            raise _error("spec_transaction_journal_temporary_changed", "Prepared journal bytes changed.")
    else:
        write_exclusive(temporary, raw)
    os.replace(temporary, path)
    return raw


def publish_journal(root: Path, journal_relative: str, marker_relative: str) -> dict[str, object]:
    journal_path = storage_path(root, journal_relative)
    marker_path = storage_path(root, marker_relative)
    raw = read_raw(journal_path)
    contract = validate_spec_transaction(raw, source=journal_relative)
    if marker_path.exists():
        marker_raw = read_raw(marker_path)
        if contract["state"] == "published" and completion_marker_matches(raw, marker_raw):
            return {"status": "already_published", "published_count": contract["published_count"]}
        raise _error("spec_transaction_marker_conflict", "The completion marker does not match the final journal.")
    files = contract["files"]
    for index in range(contract["published_count"], len(files)):
        row = files[index]
        target = storage_path(root, row["path"])
        before = _snapshot_bytes(row, "before")
        after = _snapshot_bytes(row, "after")
        current = target.read_bytes() if target.is_file() else None
        if current != after:
            if current != before:
                raise _error("spec_transaction_concurrent_change", "An artifact changed outside the transaction.")
            if after is None:
                target.unlink()
            elif before is None:
                target.parent.mkdir(parents=True, exist_ok=True)
                write_exclusive(target, after)
            else:
                temporary = Path(str(journal_path) + f".{index}.tmp")
                replace_checked(target, before, after, temporary, recover=True)
        contract["published_count"] = index + 1
        contract["state"] = "published" if index + 1 == len(files) else "publishing"
        raw = _replace_journal(journal_path, contract)
    marker_raw = raw_sha256(raw).encode("ascii") + b"\n"
    write_exclusive(marker_path, marker_raw)
    return {"status": "published", "published_count": contract["published_count"]}
