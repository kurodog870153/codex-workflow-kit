"""Stable fingerprints for reviewed TASK repair inputs and artifacts."""

from collections.abc import Iterable, Mapping

from ....technical.foundation.fingerprint import raw_sha256


def task_repair_transaction_id(request_raw: bytes) -> str:
    """Build the stable transaction identity for canonical repair request bytes."""
    return "TASK-REPAIR-" + raw_sha256(request_raw)[:12].upper()


def fingerprint_task_repair_contents(contents: Mapping[str, bytes]) -> dict[str, str]:
    """Fingerprint every available artifact in a TASK repair state."""
    return {path: raw_sha256(raw) for path, raw in contents.items()}


def fingerprint_task_repair_evidence(
    contents: Mapping[str, bytes], paths: Iterable[str]
) -> dict[str, str | None]:
    """Fingerprint the complete reviewed path set, preserving missing evidence."""
    return {
        path: raw_sha256(contents[path]) if path in contents else None
        for path in paths
    }


__all__ = [
    "fingerprint_task_repair_contents",
    "fingerprint_task_repair_evidence",
    "task_repair_transaction_id",
]
