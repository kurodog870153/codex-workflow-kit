from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.execution import REAPPROVAL_CONDITIONS
from worklib.services.attempt import minimal_authorization
from worklib.services.authorization.rules import (
    authorization_evidence,
    effective_task,
    require_deviation,
    require_modified_files,
    require_record_scope,
    require_result_evidence,
    require_retry_evidence,
    supplemental_authorizations,
)
from worklib.models.common.errors import WorkError


class AttemptAuthorizationFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        authorization = minimal_authorization()
        authorization.update(
            commands=[{"id": "CMD-001", "mode": "argv", "argv": ["tool"]}],
            validations=[{"id": "VAL-001", "command": "CMD-001"}],
            modifiable_files=["src/example.txt"],
            working_directories=["."],
            external_operations=[
                {
                    "id": "OP-001",
                    "kind": "external_state",
                    "description": "Publish the reviewed result.",
                }
            ],
            allowed_deviations=[
                {
                    "kind": "skip_record",
                    "record_id": "VAL-001",
                    "reason": "Covered by equivalent evidence.",
                }
            ],
        )
        self.attempt = {"authorization": authorization}

    def assert_code(self, expected: str, callback, *args) -> None:
        with self.assertRaises(WorkError) as context:
            callback(*args)
        self.assertEqual(context.exception.code, expected)

    def test_normal_flow_reuses_one_manifest_authorization(self) -> None:
        for record_id in ("CMD-001", "VAL-001", "OP-001"):
            require_record_scope(self.attempt, record_id)
        require_modified_files(self.attempt, ["src/example.txt"])
        require_result_evidence(
            self.attempt,
            {"id": "CMD-001", "kind": "command", "exit_code": 0},
            None,
        )
        require_result_evidence(
            self.attempt,
            {"id": "VAL-001", "kind": "validation", "outcome": "passed"},
            None,
        )
        self.assertEqual(
            authorization_evidence(self.attempt),
            "User approved this exact Attempt scope.",
        )

    def test_scope_expansion_stops_before_execution(self) -> None:
        self.assert_code(
            "execution_authorization_scope_expansion",
            require_record_scope,
            self.attempt,
            "CMD-002",
        )
        self.assert_code(
            "execution_authorization_scope_expansion",
            require_modified_files,
            self.attempt,
            ["src/unapproved.txt"],
        )

    def test_only_exact_preauthorized_deviation_is_reused(self) -> None:
        action = copy.deepcopy(self.attempt["authorization"]["allowed_deviations"][0])
        require_deviation(self.attempt, action)
        action["reason"] = "Different reason."
        self.assert_code(
            "execution_authorization_deviation_required",
            require_deviation,
            self.attempt,
            action,
        )

    def test_retry_requires_fresh_evidence_and_downstream_uses_it(self) -> None:
        self.assertIsNone(require_retry_evidence(self.attempt, "CMD-001", None))
        self.assert_code(
            "execution_authorization_retry_required",
            require_retry_evidence,
            self.attempt,
            "CMD-001#1",
            None,
        )
        self.assert_code(
            "execution_authorization_retry_evidence_reused",
            require_retry_evidence,
            self.attempt,
            "CMD-001#1",
            self.attempt["authorization"]["authorization_evidence"],
        )
        evidence = require_retry_evidence(
            self.attempt, "CMD-001#1", "User approved retry 1."
        )
        self.assertEqual(
            authorization_evidence(
                self.attempt, {"retry_authorization_evidence": evidence}
            ),
            "User approved retry 1.",
        )

    def test_failure_unknown_and_recovery_require_new_decisions(self) -> None:
        for record in (
            {"id": "CMD-001", "kind": "command", "exit_code": 1},
            {"id": "OP-001", "kind": "operation", "outcome": "failure"},
            {"id": "OP-001", "kind": "operation", "outcome": "unknown"},
            {"id": "VAL-001", "kind": "validation", "outcome": "failed"},
        ):
            with self.subTest(record=record):
                self.assert_code(
                    "execution_authorization_result_required",
                    require_result_evidence,
                    self.attempt,
                    record,
                    None,
                )
                self.assert_code(
                    "execution_authorization_result_evidence_reused",
                    require_result_evidence,
                    self.attempt,
                    record,
                    self.attempt["authorization"]["authorization_evidence"],
                )
                require_result_evidence(
                    self.attempt, record, "User reviewed this result."
                )
        self.assertIn("recovery", REAPPROVAL_CONDITIONS)

    def approved_deviation(self, action, *, blocking=False):
        return {
            "deviation_id": "DEVIATION-001",
            "approved_preview_sha256": "a" * 64,
            "proposal": {
                "anchor_record_id": "CMD-001",
                "action": action,
                "modifiable_files": [],
                "impact": {
                    "requirement_changed": blocking,
                    "scope_changed": False,
                    "acceptance_criteria_changed": False,
                    "deliverables_changed": False,
                    "safety_boundary_changed": False,
                    "external_side_effect_boundary_changed": False,
                },
            },
            "supplemental_authorization": {
                "preview_sha256": "a" * 64,
                "action": copy.deepcopy(action),
                "modifiable_files": [],
                "authorization_evidence": "Approved supplemental action.",
            },
            "decision": {
                "outcome": "approved",
                "evidence": "Approved supplemental action.",
            },
        }

    def test_approved_nonblocking_supplement_extends_effective_scope(self) -> None:
        action = {
            "kind": "add_command",
            "after_record_id": "CMD-001",
            "command": {"id": "CMD-002", "mode": "argv", "argv": ["tool", "new"]},
        }
        self.attempt["execution_deviations"] = [self.approved_deviation(action)]
        require_record_scope(self.attempt, "CMD-002")
        task = effective_task({"commands": [{"id": "CMD-001"}]}, self.attempt)
        self.assertEqual([item["id"] for item in task["commands"]], ["CMD-001", "CMD-002"])
        self.assertEqual(
            authorization_evidence(self.attempt, base_record_id="CMD-002"),
            "Approved supplemental action.",
        )

    def test_rejects_mismatch_blocking_and_duplicate_supplements(self) -> None:
        action = {
            "kind": "replace_command",
            "record_id": "CMD-001",
            "replacement": {"mode": "argv", "argv": ["tool", "new"]},
        }
        mismatch = self.approved_deviation(action)
        mismatch["supplemental_authorization"]["action"]["replacement"]["argv"] = ["other"]
        self.attempt["execution_deviations"] = [mismatch]
        self.assert_code(
            "execution_authorization_supplemental_mismatch",
            supplemental_authorizations,
            self.attempt,
        )
        self.attempt["execution_deviations"] = [
            self.approved_deviation(action, blocking=True)
        ]
        self.assert_code(
            "execution_authorization_blocking_deviation",
            supplemental_authorizations,
            self.attempt,
        )
        first = self.approved_deviation(action)
        second = copy.deepcopy(first)
        second["deviation_id"] = "DEVIATION-002"
        self.attempt["execution_deviations"] = [first, second]
        self.assert_code(
            "execution_authorization_supplemental_duplicate",
            supplemental_authorizations,
            self.attempt,
        )

    def test_modified_files_union_only_the_applicable_supplement(self) -> None:
        action = {
            "kind": "replace_command",
            "record_id": "CMD-001",
            "replacement": {"mode": "argv", "argv": ["tool", "new"]},
        }
        deviation = self.approved_deviation(action)
        deviation["proposal"]["modifiable_files"] = ["src/supplemental.py"]
        deviation["supplemental_authorization"]["modifiable_files"] = [
            "src/supplemental.py"
        ]
        self.attempt["execution_deviations"] = [deviation]
        require_modified_files(
            self.attempt,
            ["src/example.txt", "src/supplemental.py"],
            "CMD-001",
        )
        self.assert_code(
            "execution_authorization_scope_expansion",
            require_modified_files,
            self.attempt,
            ["src/supplemental.py"],
        )


if __name__ == "__main__":
    unittest.main()
