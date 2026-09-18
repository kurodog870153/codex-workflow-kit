from __future__ import annotations

from ...models.common.errors import ExitCode, WorkError


def require_known_affected_ids(contract, known_ids, *, code):
    unknown = [identifier for identifier in contract["affected_ids"] if identifier not in known_ids]
    if unknown:
        raise WorkError(
            ExitCode.CONTRACT, code,
            "Affected IDs must exist in the validated source scope.",
            {"affected_ids": unknown},
        )


def require_matching_handoff_source(contract, expected):
    mismatches = [
        field for field in ("requirement_id", "artifacts", "source")
        if contract[field] != expected[field]
    ]
    if mismatches:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY, "handoff_source_mismatch",
            "The incoming handoff does not match the selected current source.",
            {"fields": mismatches},
        )

