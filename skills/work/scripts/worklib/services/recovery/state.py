from __future__ import annotations

import copy
from typing import Any


def finished_record_index(index: dict[str, Any]) -> dict[str, Any]:
    target = copy.deepcopy(index)
    target["lock"].pop("record_id")
    target["lock"].pop("command_correction", None)
    target["lock"].pop("retry_authorization_evidence", None)
    return target


def attempt_close_request(attempt: dict[str, Any]) -> dict[str, Any]:
    request: dict[str, Any] = {
        "schema": "work-attempt-close-request/v1",
        "status": attempt["status"],
    }
    if attempt["status"] != "completed":
        request["final_type"] = attempt["final_type"]
        request["reason"] = attempt["reason"]
        request["authorization_evidence"] = attempt["closing_authorization_evidence"]
    return request
