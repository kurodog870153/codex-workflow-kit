from __future__ import annotations

import copy
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.execution.deviation import ExecutionDeviationImpactModel


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, details)


def authorization_evidence(
    attempt: dict[str, Any], lock: dict[str, Any] | None = None,
    base_record_id: str | None = None,
) -> str:
    if lock is not None and "retry_authorization_evidence" in lock:
        return lock["retry_authorization_evidence"]
    if base_record_id is not None:
        supplemental = supplemental_authorizations(attempt).get(base_record_id)
        if supplemental is not None:
            return supplemental["authorization_evidence"]
    return attempt["authorization"]["authorization_evidence"]


def _supplemental_target(action: dict[str, Any]) -> str:
    kind = action["kind"]
    if kind in {"replace_command", "skip_record"}:
        return action["record_id"].split("#", 1)[0]
    if kind == "add_command":
        return action["command"]["id"]
    if kind == "add_validation":
        return action["validation"]["id"]
    return action["operation"]["id"]


def supplemental_authorizations(attempt: dict[str, Any]) -> dict[str, dict[str, Any]]:
    resolved: dict[str, dict[str, Any]] = {}
    for deviation in attempt.get("execution_deviations", []):
        if deviation.get("decision", {}).get("outcome") != "approved":
            continue
        proposal = deviation.get("proposal", {})
        supplemental = deviation.get("supplemental_authorization", {})
        action = proposal.get("action")
        if (
            supplemental.get("preview_sha256") != deviation.get("approved_preview_sha256")
            or supplemental.get("action") != action
            or supplemental.get("modifiable_files", [])
            != proposal.get("modifiable_files", [])
            or supplemental.get("authorization_evidence")
            != deviation.get("decision", {}).get("evidence")
        ):
            _fail(
                "execution_authorization_supplemental_mismatch",
                "The supplemental authorization does not match its approved deviation.",
                deviation_id=deviation.get("deviation_id"),
            )
        if ExecutionDeviationImpactModel.crosses_semantic_boundary(proposal["impact"]):
            _fail(
                "execution_authorization_blocking_deviation",
                "A blocking deviation cannot extend the active Attempt authorization.",
                deviation_id=deviation.get("deviation_id"),
            )
        anchor = proposal.get("anchor_record_id")
        base_anchor = anchor.split("#", 1)[0] if isinstance(anchor, str) else None
        kind = action.get("kind") if isinstance(action, dict) else None
        anchored = (
            kind == "replace_command" and action.get("record_id") == anchor
            or kind == "skip_record" and action.get("record_id") == anchor
            or kind == "add_command" and action.get("after_record_id") == anchor
            or kind == "adjust_operation"
            and action.get("operation", {}).get("id") == base_anchor
            or kind == "add_validation"
        )
        if not anchored:
            _fail(
                "execution_authorization_supplemental_anchor",
                "The supplemental action does not match its approved anchor.",
                deviation_id=deviation.get("deviation_id"),
            )
        target = _supplemental_target(action)
        if target in resolved:
            _fail(
                "execution_authorization_supplemental_duplicate",
                "More than one supplemental action targets the same record.",
                record_id=target,
            )
        resolved[target] = supplemental
    return resolved


def effective_task(task: dict[str, Any], attempt: dict[str, Any]) -> dict[str, Any]:
    effective = copy.deepcopy(task)
    supplemental = supplemental_authorizations(attempt)
    for authorization in supplemental.values():
        action = authorization["action"]
        kind = action["kind"]
        if kind == "replace_command":
            record_id = action["record_id"].split("#", 1)[0]
            replacement = {"id": record_id, **copy.deepcopy(action["replacement"])}
            effective["commands"] = [
                replacement if item["id"] == record_id else item
                for item in effective.get("commands", [])
            ]
        elif kind == "add_command":
            effective.setdefault("commands", []).append(copy.deepcopy(action["command"]))
        elif kind == "add_validation":
            effective.setdefault("validations", []).append(
                copy.deepcopy(action["validation"])
            )
        elif kind == "adjust_operation":
            operation = copy.deepcopy(action["operation"])
            effective["operations"] = [
                operation if item["id"] == operation["id"] else item
                for item in effective.get("operations", [])
            ]
    return effective


def require_record_scope(attempt: dict[str, Any], base_record_id: str) -> None:
    prefix, field = base_record_id[:3], None
    if prefix == "CMD":
        field = "commands"
    elif prefix == "VAL":
        field = "validations"
    elif prefix == "OP-":
        field = "external_operations"
    original = field is not None and any(
        item["id"] == base_record_id for item in attempt["authorization"][field]
    )
    if not original and base_record_id not in supplemental_authorizations(attempt):
        _fail(
            "execution_authorization_scope_expansion",
            "The record is outside the authorized Attempt scope.",
            record_id=base_record_id,
        )


def require_retry_evidence(
    attempt: dict[str, Any], record_id: str, evidence: str | None
) -> str | None:
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
    if evidence == attempt["authorization"]["authorization_evidence"]:
        _fail(
            "execution_authorization_retry_evidence_reused",
            "A retry cannot reuse the original Attempt authorization evidence.",
            record_id=record_id,
        )
    return evidence


def require_modified_files(
    attempt: dict[str, Any], paths: list[str], base_record_id: str | None = None
) -> None:
    allowed = set(attempt["authorization"]["modifiable_files"])
    if base_record_id is not None:
        supplemental = supplemental_authorizations(attempt).get(base_record_id)
        if supplemental is not None:
            allowed.update(supplemental.get("modifiable_files", []))
    outside = sorted(set(paths) - allowed)
    if outside:
        _fail(
            "execution_authorization_scope_expansion",
            "Modified files are outside the authorized Attempt scope.",
            files=outside,
        )


def require_result_evidence(
    attempt: dict[str, Any], record: dict[str, Any], evidence: str | None
) -> None:
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
    if (
        failed
        and evidence == attempt["authorization"]["authorization_evidence"]
    ):
        _fail(
            "execution_authorization_result_evidence_reused",
            "A failed or unknown result cannot reuse the original Attempt authorization evidence.",
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
