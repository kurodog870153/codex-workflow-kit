from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.recovery_models import ExecutionRecoveryRequestContract
from worklib.foundation.errors import WorkError


class ExecutionRecoveryRequestContractTests(unittest.TestCase):
    def parse(self, value: object) -> dict[str, object]:
        return ExecutionRecoveryRequestContract.parse_request(
            json.dumps(value).encode(), source="test"
        ).to_canonical_dict()

    def request(self) -> dict[str, object]:
        return {
            "schema": "work-execution-recovery-request/v1",
            "transaction": "record_begin",
            "attempt_id": "ATTEMPT-001",
            "transaction_files": ["prepared.tmp"],
        }

    def test_parses_canonical_request(self) -> None:
        request = self.request()
        self.assertEqual(self.parse(request), request)

    def test_preserves_request_error_codes(self) -> None:
        cases = (
            ({**self.request(), "schema": "invalid"}, "execution_recovery_invalid_schema"),
            ({**self.request(), "transaction": "invalid"}, "execution_recovery_invalid_transaction"),
            ({**self.request(), "attempt_id": "ATTEMPT-1"}, "execution_recovery_invalid_attempt_id"),
            ({**self.request(), "transaction_files": "bad"}, "execution_recovery_invalid_file_list"),
            ({**self.request(), "transaction_files": ["a/b"]}, "execution_recovery_invalid_file_name"),
            ({**self.request(), "transaction_files": ["b", "a"]}, "execution_recovery_noncanonical_file_list"),
        )
        for request, expected in cases:
            with self.subTest(expected=expected):
                with self.assertRaises(WorkError) as context:
                    self.parse(request)
                self.assertEqual(context.exception.code, expected)


if __name__ == "__main__":
    unittest.main()
