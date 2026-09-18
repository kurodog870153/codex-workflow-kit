from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.attempt_authorization_models import (
    REAPPROVAL_CONDITIONS,
    minimal_authorization,
)
from worklib.execution.authorization import (
    authorization_evidence,
    require_deviation,
    require_modified_files,
    require_record_scope,
    require_result_evidence,
    require_retry_evidence,
)
from worklib.foundation.errors import WorkError


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
            {"id": "CMD-001", "kind": "command", "exit_code": 0}, None
        )
        require_result_evidence(
            {"id": "VAL-001", "kind": "validation", "outcome": "passed"}, None
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
        self.assertIsNone(require_retry_evidence("CMD-001", None))
        self.assert_code(
            "execution_authorization_retry_required",
            require_retry_evidence,
            "CMD-001#1",
            None,
        )
        evidence = require_retry_evidence("CMD-001#1", "User approved retry 1.")
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
                    record,
                    None,
                )
                require_result_evidence(record, "User reviewed this result.")
        self.assertIn("recovery", REAPPROVAL_CONDITIONS)


if __name__ == "__main__":
    unittest.main()
