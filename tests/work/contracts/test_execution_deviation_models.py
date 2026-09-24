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
    ExecutionDeviationAuthorizationContract,
    ExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract,
    ExecutionDeviationRecordContract,
    ExecutionDeviationSemanticRequestContract,
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
            ExecutionDeviationAuthorizationContract,
            ExecutionDeviationSemanticRequestContract,
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
                if action["kind"] in {"skip_record", "adjust_operation"}:
                    proposal["anchor_record_id"] = "OP-001"
                    proposal["task_basis"] = ["OP-001", "STEP-001"]
                proposal["action"] = action
                parsed = ExecutionDeviationProposalContract.model_validate(proposal)
                self.assertEqual(parsed.action.kind, action["kind"])

    def test_semantic_actions_reject_formal_ids_and_references(self) -> None:
        example = copy.deepcopy(ExecutionDeviationSemanticRequestContract.contract_example)
        actions = (
            {"kind": "replace_command", "record_id": "CMD-001", "replacement": {"mode": "argv", "argv": ["tool"]}},
            {"kind": "skip_record", "record_id": "CMD-001", "reason": "Skip."},
            {"kind": "add_command", "after_record_id": "CMD-001", "command": {"mode": "argv", "argv": ["tool"]}},
            {"kind": "add_command", "command": {"id": "CMD-002", "mode": "argv", "argv": ["tool"]}},
            {"kind": "add_validation", "validation": {"id": "VAL-002", "kind": "manual", "confirmer": "user", "criteria": "Reviewed."}},
            {"kind": "add_validation", "validation": {"kind": "automated", "command_ids": ["CMD-001"]}},
            {"kind": "adjust_operation", "operation": {"id": "OP-001", "kind": "file", "action": "Update.", "target": "src/app.py", "validation_position": 1}},
            {"kind": "adjust_operation", "operation": {"kind": "file", "action": "Update.", "target": "src/app.py", "validation_id": "VAL-001"}},
        )
        for action in actions:
            with self.subTest(action=action):
                example["action"] = action
                with self.assertRaises(ValidationError):
                    ExecutionDeviationSemanticRequestContract.model_validate(example)

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

    def test_accepts_semantic_boundary_changes_and_rejects_invalid_command_shapes(self) -> None:
        boundary = self.proposal()
        boundary["impact"]["requirement_changed"] = True
        parsed = ExecutionDeviationProposalContract.model_validate(boundary)
        self.assertTrue(parsed.impact.requirement_changed)
        scope = self.proposal()
        scope["impact"]["scope_changed"] = True
        self.assertTrue(
            ExecutionDeviationProposalContract.model_validate(scope).impact.scope_changed
        )
        invalid_command = self.proposal()
        invalid_command["action"]["replacement"] = {
            "mode": "argv", "argv": ["tool"], "script": "tool",
        }
        with self.assertRaises(ValidationError):
            ExecutionDeviationProposalContract.model_validate(invalid_command)

    def test_rejected_decision_requires_not_needed_reconciliation(self) -> None:
        artifact = self.artifact()
        artifact["decision"] = {"outcome": "rejected", "evidence": "User rejected the proposal."}
        artifact["supplemental_authorization"]["authorization_evidence"] = "User rejected the proposal."
        artifact["reconciliation_status"] = "pending"

        with self.assertRaises(ValidationError):
            ExecutionDeviationContract.model_validate(artifact)

        artifact["reconciliation_status"] = "not_needed"
        parsed = ExecutionDeviationContract.model_validate(artifact)
        self.assertEqual(parsed.reconciliation_status, "not_needed")

    def test_approved_decision_remains_reconcilable(self) -> None:
        artifact = self.artifact()
        artifact["reconciliation_status"] = "not_needed"
        with self.assertRaises(ValidationError):
            ExecutionDeviationContract.model_validate(artifact)

    def test_proposal_action_and_basis_bind_the_anchor(self) -> None:
        cases = []
        missing_basis = self.proposal()
        missing_basis["task_basis"] = ["STEP-001"]
        cases.append(missing_basis)
        wrong_target = self.proposal()
        wrong_target["action"]["record_id"] = "CMD-002"
        cases.append(wrong_target)
        wrong_position = self.proposal()
        wrong_position["action"] = {
            "kind": "add_command",
            "after_record_id": "CMD-002",
            "command": {"id": "CMD-003", "mode": "argv", "argv": ["tool"]},
        }
        cases.append(wrong_position)
        wrong_operation = self.proposal()
        wrong_operation["anchor_record_id"] = "OP-001"
        wrong_operation["task_basis"] = ["OP-001"]
        wrong_operation["action"] = {
            "kind": "adjust_operation",
            "operation": {
                "id": "OP-002", "kind": "file", "action": "Update.",
                "target": "src/app.py", "validation_id": "VAL-001",
            },
        }
        cases.append(wrong_operation)
        for proposal in cases:
            with self.subTest(action=proposal["action"]):
                with self.assertRaises(ValidationError):
                    ExecutionDeviationProposalContract.model_validate(proposal)

    def test_supplemental_authorization_binds_preview_action_and_fresh_evidence(self) -> None:
        artifact = self.artifact()
        authorization = artifact["supplemental_authorization"]
        self.assertEqual(authorization["preview_sha256"], artifact["approved_preview_sha256"])
        self.assertEqual(authorization["action"], artifact["proposal"]["action"])
        self.assertNotEqual(
            authorization["authorization_evidence"],
            "User approved this exact Attempt scope.",
        )

    def test_artifact_rejects_cross_field_authorization_mismatches(self) -> None:
        cases = []
        fingerprint = self.artifact()
        fingerprint["supplemental_authorization"]["preview_sha256"] = "d" * 64
        cases.append(fingerprint)
        action = self.artifact()
        action["supplemental_authorization"]["action"] = {
            "kind": "replace_command",
            "record_id": "CMD-001",
            "replacement": {"mode": "argv", "argv": ["other"]},
        }
        cases.append(action)
        evidence = self.artifact()
        evidence["decision"]["evidence"] = "Different evidence."
        cases.append(evidence)
        for artifact in cases:
            with self.subTest(artifact=artifact):
                with self.assertRaises(ValidationError):
                    ExecutionDeviationContract.model_validate(artifact)

    def test_supplemental_file_scope_is_preview_bound_and_defaults_empty(self) -> None:
        proposal = self.proposal()
        proposal.pop("modifiable_files")
        self.assertEqual(
            ExecutionDeviationProposalContract.model_validate(proposal).modifiable_files,
            [],
        )
        artifact = self.artifact()
        artifact["proposal"]["modifiable_files"] = ["src/extra.py"]
        artifact["supplemental_authorization"]["modifiable_files"] = ["src/extra.py"]
        parsed = ExecutionDeviationContract.model_validate(artifact)
        self.assertEqual(
            parsed.supplemental_authorization.modifiable_files, ["src/extra.py"]
        )
        artifact["supplemental_authorization"]["modifiable_files"] = []
        with self.assertRaises(ValidationError):
            ExecutionDeviationContract.model_validate(artifact)

    def test_supplemental_file_scope_rejects_unsafe_or_duplicate_paths(self) -> None:
        for paths in (["../outside.py"], ["src\\file.py"], ["src/a.py", "src/a.py"]):
            with self.subTest(paths=paths):
                proposal = self.proposal()
                proposal["modifiable_files"] = paths
                with self.assertRaises(ValidationError):
                    ExecutionDeviationProposalContract.model_validate(proposal)

    def test_preview_classification_and_blocking_follow_impact(self) -> None:
        preview = copy.deepcopy(ExecutionDeviationPreviewContract.contract_example)
        preview["proposal"]["impact"]["scope_changed"] = True
        with self.assertRaises(ValidationError):
            ExecutionDeviationPreviewContract.model_validate(preview)
        preview["classification"] = "plan_and_task"
        preview["blocking"] = True
        self.assertTrue(ExecutionDeviationPreviewContract.model_validate(preview).blocking)

    def test_record_classification_blocking_and_path_are_consistent(self) -> None:
        record = copy.deepcopy(ExecutionDeviationRecordContract.contract_example)
        record["blocking"] = True
        with self.assertRaises(ValidationError):
            ExecutionDeviationRecordContract.model_validate(record)
        record = copy.deepcopy(ExecutionDeviationRecordContract.contract_example)
        record["attempt_path"] = record["attempt_path"].replace("TASK-001", "TASK-002")
        with self.assertRaises(ValidationError):
            ExecutionDeviationRecordContract.model_validate(record)


if __name__ == "__main__":
    unittest.main()
