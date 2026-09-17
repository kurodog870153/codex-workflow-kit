from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from pydantic import ValidationError

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.execution_inspection_models import (
    ExecutePreflightContract,
    ExecuteWorktreeContract,
    ExecuteWorktreeSnapshotContract,
)


class ExecutionInspectionContractTests(unittest.TestCase):
    def test_examples_round_trip_canonically(self) -> None:
        for contract in (ExecutePreflightContract, ExecuteWorktreeContract, ExecuteWorktreeSnapshotContract):
            with self.subTest(contract=contract.contract_id):
                model = contract.model_validate(copy.deepcopy(contract.contract_example))
                parsed = contract.parse_json_bytes(model.render_canonical_json(), source="test")
                self.assertEqual(parsed.to_canonical_dict(), contract.contract_example)

    def test_public_contracts_reject_extra_fields_and_coercion(self) -> None:
        for contract in (ExecutePreflightContract, ExecuteWorktreeContract):
            with self.subTest(contract=contract.contract_id):
                value = copy.deepcopy(contract.contract_example)
                value["extra"] = True
                with self.assertRaises(ValidationError):
                    contract.model_validate(value)
        value = copy.deepcopy(ExecuteWorktreeContract.contract_example)
        value["excluded_execution_change_count"] = "0"
        with self.assertRaises(ValidationError):
            ExecuteWorktreeContract.model_validate(value)


if __name__ == "__main__":
    unittest.main()
