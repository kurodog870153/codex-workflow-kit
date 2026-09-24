"""Reject caller-authored formal evidence inside semantic nested values."""

from __future__ import annotations

import re
from typing import Any


FORMAL_KEYS = {
    "id", "command_ids", "command_id", "validation_id", "acceptance_ids",
    "goal_ids", "deliverable_ids", "milestone_ids", "task_ids",
    "canonical_sha256", "raw_sha256", "instructions_sha256", "fingerprint",
    "revision", "status", "transaction_id", "spec_id", "change_id",
}
FORMAL_ID = re.compile(
    r"(?:TASK|STEP|CMD|VAL|OP|FILE|INPUT|RISK|GOAL|SCOPE|CONSTRAINT|"
    r"DEPENDENCY|MILESTONE|DELIVERABLE|ACCEPTANCE|PLAN-DECISION|TASK-DECISION|"
    r"PLAN-CHANGE|TASK-CHANGE|TASK-SPEC)-[0-9]{3}"
)


class SemanticEvidencePolicy:
    @staticmethod
    def reject_formal_data(value: Any) -> None:
        if isinstance(value, dict):
            if set(value) & FORMAL_KEYS:
                raise ValueError("Semantic input cannot contain formal fields.")
            for child in value.values():
                SemanticEvidencePolicy.reject_formal_data(child)
        elif isinstance(value, list):
            for child in value:
                SemanticEvidencePolicy.reject_formal_data(child)
        elif isinstance(value, str) and FORMAL_ID.fullmatch(value):
            raise ValueError("Semantic input cannot contain formal IDs.")
