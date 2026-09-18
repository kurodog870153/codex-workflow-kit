from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = REPO_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.execution.attempt import AttemptValidationContract
from worklib.services.attempt.validation import (
    canonicalize_attempt_contract,
    validate_attempt_contract,
)
from worklib.models.common.errors import WorkError
from worklib.models.execution import ExecutionDeviationContract
from worklib.services.attempt import authorization_sha256, minimal_authorization


class AttemptContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.attempt = {
            "schema": "work-attempt/v1",
            "attempt_id": "ATTEMPT-001",
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "status": "in_progress",
            "task_collection_sha256": "a" * 64,
            "task_index_sha256": "1" * 64,
            "task_item_sha256": "2" * 64,
            "task_instructions_sha256": "b" * 64,
            "execute_instructions_sha256": "c" * 64,
            "hierarchy_selection_sha256": "f" * 64,
            "execute_skill_selection_sha256": "d" * 64,
            "authorization": minimal_authorization(),
            "authorization_sha256": authorization_sha256(minimal_authorization()),
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
                "closing_authorization_evidence": "User approved this closure.",
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
                "closing_authorization_evidence": "User approved this closure.",
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

    def test_accepts_v1_collection_fingerprints(self) -> None:
        result = validate_attempt_contract(self.attempt, project_root=REPO_ROOT)

        self.assertEqual(result["schema"], "work-attempt-validation/v1")
        self.assertEqual(
            AttemptValidationContract.model_validate(result).to_canonical_dict(),
            result,
        )

    def test_v1_rejects_legacy_single_file_fingerprint(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt["task_sha256"] = attempt.pop("task_collection_sha256")
        attempt.pop("task_index_sha256")
        attempt.pop("task_item_sha256")

        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)

        self.assertEqual(context.exception.code, "attempt_invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            [
                "task_collection_sha256",
                "task_index_sha256",
                "task_item_sha256",
            ],
        )
        self.assertEqual(context.exception.details["unknown"], ["task_sha256"])

    def test_rejects_retired_schema(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt["schema"] = "work-attempt/v2"

        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)

        self.assertEqual(context.exception.code, "attempt_invalid_schema")

    def test_canonicalizes_execution_deviations_and_rejects_noncontiguous_ids(self) -> None:
        attempt = copy.deepcopy(self.attempt)
        attempt["execution_deviations"] = [
            copy.deepcopy(ExecutionDeviationContract.contract_example)
        ]
        canonical = canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)
        self.assertEqual(
            canonical["execution_deviations"][0]["deviation_id"], "DEVIATION-001"
        )
        attempt["execution_deviations"][0]["deviation_id"] = "DEVIATION-002"
        with self.assertRaises(WorkError) as context:
            canonicalize_attempt_contract(attempt, project_root=REPO_ROOT)
        self.assertEqual(
            context.exception.code, "attempt_invalid_execution_deviation_sequence"
        )


if __name__ == "__main__":
    unittest.main()
