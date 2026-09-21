from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from pydantic import ValidationError


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.execution import (
    ExecutionDeviationContract,
    ExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract,
    ExecutionDeviationRecordContract,
)
from worklib.models.execution import (
    ExecutionDeviationContract as ModelExecutionDeviationContract,
    ExecutionDeviationPreviewContract as ModelExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract as ModelExecutionDeviationProposalContract,
    ExecutionDeviationRecordContract as ModelExecutionDeviationRecordContract,
)


class ExecutionDeviationContractTests(unittest.TestCase):
    def test_legacy_exports_preserve_class_identity(self) -> None:
        self.assertIs(ExecutionDeviationContract, ModelExecutionDeviationContract)
        self.assertIs(ExecutionDeviationPreviewContract, ModelExecutionDeviationPreviewContract)
        self.assertIs(ExecutionDeviationProposalContract, ModelExecutionDeviationProposalContract)
        self.assertIs(ExecutionDeviationRecordContract, ModelExecutionDeviationRecordContract)

    def proposal(self) -> dict[str, object]:
        return copy.deepcopy(ExecutionDeviationProposalContract.contract_example)

    def artifact(self) -> dict[str, object]:
        return copy.deepcopy(ExecutionDeviationContract.contract_example)

    def test_examples_validate_and_use_canonical_order(self) -> None:
        for contract in (
            ExecutionDeviationProposalContract, ExecutionDeviationContract,
            ExecutionDeviationPreviewContract, ExecutionDeviationRecordContract,
        ):
            with self.subTest(contract=contract.contract_id):
                model = contract.model_validate(copy.deepcopy(contract.contract_example))
                self.assertEqual(list(model.to_canonical_dict()), list(contract.canonical_order))

    def test_accepts_each_strict_action_shape(self) -> None:
        actions = (
            {"kind": "replace_command", "record_id": "CMD-001", "replacement": {"mode": "shell", "script": "tool test"}},
            {"kind": "add_command", "after_record_id": "CMD-001", "command": {"id": "CMD-002", "mode": "argv", "argv": ["tool", "test"]}},
            {"kind": "add_validation", "validation": {"id": "VAL-002", "kind": "manual", "confirmer": "user", "criteria": "Reviewed."}},
            {"kind": "skip_record", "record_id": "OP-001", "reason": "The operation is not applicable."},
            {"kind": "adjust_operation", "operation": {"id": "OP-001", "kind": "file", "action": "Update the configured target.", "target": "src/app.py", "validation_id": "VAL-001"}},
        )
        for action in actions:
            with self.subTest(kind=action["kind"]):
                proposal = self.proposal()
                proposal["action"] = action
                parsed = ExecutionDeviationProposalContract.model_validate(proposal)
                self.assertEqual(parsed.action.kind, action["kind"])

    def test_rejects_missing_unknown_and_mixed_action_fields(self) -> None:
        cases = []
        missing = self.proposal()
        missing.pop("gap")
        cases.append(missing)
        unknown = self.proposal()
        unknown["unexpected"] = True
        cases.append(unknown)
        mixed = self.proposal()
        mixed["action"]["reason"] = "Not allowed for replace_command."
        cases.append(mixed)
        for proposal in cases:
            with self.subTest(proposal=proposal):
                with self.assertRaises(ValidationError):
                    ExecutionDeviationProposalContract.model_validate(proposal)

    def test_rejects_semantic_boundary_changes_and_invalid_command_shapes(self) -> None:
        boundary = self.proposal()
        boundary["impact"]["requirement_changed"] = True
        invalid_command = self.proposal()
        invalid_command["action"]["replacement"] = {
            "mode": "argv", "argv": ["tool"], "script": "tool",
        }
        for proposal in (boundary, invalid_command):
            with self.subTest(proposal=proposal):
                with self.assertRaises(ValidationError):
                    ExecutionDeviationProposalContract.model_validate(proposal)

    def test_rejected_decision_requires_not_needed_reconciliation(self) -> None:
        artifact = self.artifact()
        artifact["decision"] = {"outcome": "rejected", "evidence": "User rejected the proposal."}
        artifact["reconciliation_status"] = "pending"

        with self.assertRaises(ValidationError):
            ExecutionDeviationContract.model_validate(artifact)

        artifact["reconciliation_status"] = "not_needed"
        parsed = ExecutionDeviationContract.model_validate(artifact)
        self.assertEqual(parsed.reconciliation_status, "not_needed")


if __name__ == "__main__":
    unittest.main()
