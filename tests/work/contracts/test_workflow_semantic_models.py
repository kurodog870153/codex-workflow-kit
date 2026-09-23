from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.models.plan import PlanSemanticRequestContract
from worklib.models.task_draft import TaskSemanticRequestContract
from pydantic import ValidationError

from worklib.models.workflow import (
    OperationEnvelopeContract, OperationResultContract, WorkflowStateContract,
)


class WorkflowSemanticModelTests(unittest.TestCase):
    def test_semantic_requests_exclude_derived_machine_fields(self):
        plan = PlanSemanticRequestContract.model_validate(PlanSemanticRequestContract.contract_example)
        task = TaskSemanticRequestContract.model_validate(TaskSemanticRequestContract.contract_example)
        self.assertNotIn("artifacts", plan.to_canonical_dict())
        self.assertNotIn("id", task.to_canonical_dict()["upsert"][0])

    def test_workflow_state_preserves_confirmation_gate(self):
        state = WorkflowStateContract.model_validate(WorkflowStateContract.contract_example)
        self.assertTrue(state.requires_user_confirmation)
        self.assertEqual(state.next_action, "prepare_plan")

    def test_operation_envelope_requires_fingerprint_bound_context(self):
        envelope = OperationEnvelopeContract.model_validate(
            OperationEnvelopeContract.contract_example
        )
        self.assertEqual(envelope.side_effect_boundary, "read_only")
        incomplete = dict(OperationEnvelopeContract.contract_example)
        del incomplete["selection_sha256"]
        with self.assertRaises(ValidationError):
            OperationEnvelopeContract.model_validate(incomplete)

    def test_operation_result_is_bound_to_one_context(self):
        result = OperationResultContract.model_validate(OperationResultContract.contract_example)
        self.assertEqual(result.status, "success")
        self.assertEqual(result.context_sha256, "0" * 64)


if __name__ == "__main__":
    unittest.main()
