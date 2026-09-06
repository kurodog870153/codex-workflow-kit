from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.attempt import canonicalize_attempt_contract
from worklib.foundation.errors import WorkError


class AttemptContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.attempt = {
            "schema": "work-attempt/v1",
            "attempt_id": "ATTEMPT-001",
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "status": "in_progress",
            "task_sha256": "a" * 64,
            "task_instructions_sha256": "b" * 64,
            "execute_instructions_sha256": "c" * 64,
            "hierarchy_selection_sha256": "f" * 64,
            "execute_skill_selection_sha256": "d" * 64,
            "started_at": "2026-09-01T10:00+08:00",
            "records": [],
        }

    def test_accepts_instructions_changed(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt.update(
            {
                "status": "stopped",
                "final_type": "instructions_changed",
                "reason": "The Execute instructions changed.",
                "ended_at": "2026-09-01T10:05+08:00",
            }
        )

        canonical = canonicalize_attempt_contract(
            attempt,
            project_root=REPO_ROOT,
        )

        self.assertEqual(canonical["final_type"], "instructions_changed")

    def test_rejects_legacy_rules_changed(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt.update(
            {
                "status": "stopped",
                "final_type": "rules_changed",
                "reason": "Legacy reason.",
                "ended_at": "2026-09-01T10:05+08:00",
            }
        )

        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)

        self.assertEqual(context.exception.code, "attempt_invalid_final_type")

    def test_rejects_legacy_rule_fingerprints(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt["task_rules_sha256"] = attempt.pop("task_instructions_sha256")
        attempt["execute_rules_sha256"] = attempt.pop(
            "execute_instructions_sha256"
        )

        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)

        self.assertEqual(context.exception.code, "attempt_invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["execute_instructions_sha256", "task_instructions_sha256"],
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["execute_rules_sha256", "task_rules_sha256"],
        )


if __name__ == "__main__":
    unittest.main()
