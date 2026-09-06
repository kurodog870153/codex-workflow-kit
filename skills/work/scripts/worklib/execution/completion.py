from __future__ import annotations

from typing import Any

from ..foundation.errors import ExitCode, WorkError


def _latest_record_outcomes(attempt: dict[str, Any]) -> dict[str, str]:
    outcomes: dict[str, str] = {}
    for carried in attempt.get("carried_records", []):
        base_id = carried["record_id"].split("#", 1)[0]
        if base_id.startswith("VAL-"):
            outcomes[base_id] = "passed"
    for record in attempt["records"]:
        base_id = record["id"].split("#", 1)[0]
        if record["kind"] == "validation":
            outcomes[base_id] = record["outcome"]
    return outcomes


def validate_completed_coverage(
    *, task: dict[str, Any], attempt: dict[str, Any]
) -> None:
    required = [item["id"] for item in task.get("validations", [])]
    outcomes = _latest_record_outcomes(attempt)
    missing = [record_id for record_id in required if record_id not in outcomes]
    failed = [
        record_id
        for record_id in required
        if outcomes.get(record_id) not in {None, "passed"}
    ]
    if missing or failed:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "attempt_close_incomplete_validations",
            "A completed Attempt requires every formal validation to pass.",
            {"missing": missing, "failed": failed},
        )
