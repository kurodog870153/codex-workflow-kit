from __future__ import annotations

from typing import Any

from ..foundation.errors import ExitCode, WorkError


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, details)


def authorization_evidence(
    attempt: dict[str, Any], lock: dict[str, Any] | None = None
) -> str:
    if lock is not None and "retry_authorization_evidence" in lock:
        return lock["retry_authorization_evidence"]
    return attempt["authorization"]["authorization_evidence"]


def require_record_scope(attempt: dict[str, Any], base_record_id: str) -> None:
    prefix, field = base_record_id[:3], None
    if prefix == "CMD":
        field = "commands"
    elif prefix == "VAL":
        field = "validations"
    elif prefix == "OP-":
        field = "external_operations"
    if field is None or not any(
        item["id"] == base_record_id for item in attempt["authorization"][field]
    ):
        _fail(
            "execution_authorization_scope_expansion",
            "The record is outside the authorized Attempt scope.",
            record_id=base_record_id,
        )


def require_retry_evidence(record_id: str, evidence: str | None) -> str | None:
    if "#" not in record_id:
        if evidence is not None:
            _fail(
                "execution_authorization_unexpected_retry_evidence",
                "Normal record execution reuses the Attempt authorization.",
            )
        return None
    if not isinstance(evidence, str) or not evidence.strip():
        _fail(
            "execution_authorization_retry_required",
            "A retry requires fresh authorization evidence.",
            record_id=record_id,
        )
    return evidence


def require_modified_files(attempt: dict[str, Any], paths: list[str]) -> None:
    allowed = set(attempt["authorization"]["modifiable_files"])
    outside = sorted(set(paths) - allowed)
    if outside:
        _fail(
            "execution_authorization_scope_expansion",
            "Modified files are outside the authorized Attempt scope.",
            files=outside,
        )


def require_result_evidence(record: dict[str, Any], evidence: str | None) -> None:
    kind = record.get("kind")
    failed = (
        (kind == "command" and record.get("exit_code") != 0)
        or (kind == "operation" and record.get("outcome") in {"failure", "unknown"})
        or (kind == "validation" and record.get("outcome") == "failed")
    )
    if failed and (not isinstance(evidence, str) or not evidence.strip()):
        _fail(
            "execution_authorization_result_required",
            "A failed or unknown result requires fresh authorization evidence.",
            record_id=record.get("id"),
        )
    if not failed and evidence is not None:
        _fail(
            "execution_authorization_unexpected_result_evidence",
            "A successful result reuses the Attempt authorization.",
            record_id=record.get("id"),
        )


def require_deviation(attempt: dict[str, Any], action: dict[str, Any]) -> None:
    if action not in attempt["authorization"]["allowed_deviations"]:
        _fail(
            "execution_authorization_deviation_required",
            "The deviation is outside the preauthorized actions.",
            action=action,
        )
