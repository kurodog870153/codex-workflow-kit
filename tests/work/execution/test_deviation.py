from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.execution import ExecutionDeviationProposalContract, ExecutionDeviationSemanticRequestContract
from worklib.services.attempt import authorization_sha256, minimal_authorization
from worklib.orchestration.execution import prepare_execution_deviation, prepare_semantic_execution_deviation, record_execution_deviation
from worklib.models.common.errors import WorkError


class DeviationPreviewTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.execution = "outputs/work/executions/example"
        self.task_path = "outputs/work/tasks/example/index.json"
        self.plan_path = "outputs/work/plans/example.json"
        self.attempt_path = self.execution + "/TASK-001/ATTEMPT-001/attempt.json"
        for relative, content in ((self.task_path, b"task"), (self.plan_path, b"plan"),
                                  (self.execution + "/index.json", b"index"), (self.attempt_path, b"attempt")):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        self.task = {"id": "TASK-001", "steps": [{"id": "STEP-001"}],
                     "commands": [{"id": "CMD-001"}], "operations": [], "validations": [],
                     "traceability": {"acceptance_ids": ["ACCEPTANCE-001"]}}
        self.contract = {"artifacts": {"plan": self.plan_path, "task": self.task_path,
                         "execution": self.execution}, "tasks": [self.task]}
        self.validation = {}
        self.index = {"lock": {"kind": "execution", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
                      "record_id": "CMD-001", "execute_instructions_sha256": "a" * 64}}
        authorization = minimal_authorization()
        authorization["allowed_deviations"] = [copy.deepcopy(
            ExecutionDeviationProposalContract.contract_example["action"]
        )]
        self.attempt = {
            "status": "in_progress",
            "execute_instructions_sha256": "a" * 64,
            "authorization": authorization,
        }
        self.row = {"status": "in_progress", "latest_attempt": "ATTEMPT-001"}
        self.proposal = copy.deepcopy(ExecutionDeviationProposalContract.contract_example)

    def snapshot(self) -> dict[str, bytes]:
        return {str(path): path.read_bytes() for path in self.root.rglob("*") if path.is_file()}

    def prepare(self, sources=None):
        context = {"contract": self.contract, "validation": self.validation,
                   "sources": sources or {self.task_path: b"task"}}
        with patch("worklib.orchestration.execution.ExecutionOperations.load_task_execution_context", return_value=context), \
             patch("worklib.business_services.execution.deviation._validate_index_bytes", return_value=self.index), \
             patch("worklib.business_services.execution.deviation._validate_attempt_bytes", return_value=self.attempt), \
             patch("worklib.business_services.execution.deviation.validate_execution_identity", return_value=self.row), \
             patch("worklib.business_services.execution.deviation.validate_execute_instructions"), \
             patch("worklib.business_services.execution.deviation.require_idle_writer"):
            return prepare_execution_deviation(json.dumps(self.proposal).encode(), source="test",
                project_root=self.root, user_config_root=str(self.root), raw_task_path=self.task_path,
                raw_execution_dir=self.execution, task_id="TASK-001")

    def prepare_semantic(self, action=None):
        context = {"contract": self.contract, "validation": self.validation,
                   "sources": {self.task_path: b"task"}}
        semantic = {field: self.proposal[field] for field in
                    ("gap", "action", "modifiable_files", "impact", "side_effects")}
        semantic["action"] = action or copy.deepcopy(ExecutionDeviationSemanticRequestContract.contract_example["action"])
        with patch("worklib.orchestration.execution.ExecutionOperations.load_task_execution_context", return_value=context), \
             patch("worklib.business_services.execution.deviation._validate_index_bytes", return_value=self.index), \
             patch("worklib.business_services.execution.deviation._validate_attempt_bytes", return_value=self.attempt), \
             patch("worklib.business_services.execution.deviation.validate_execution_identity", return_value=self.row), \
             patch("worklib.business_services.execution.deviation.validate_execute_instructions"), \
             patch("worklib.business_services.execution.deviation.require_idle_writer"):
            return prepare_semantic_execution_deviation(json.dumps(semantic).encode(), source="test",
                project_root=self.root, user_config_root=str(self.root), raw_task_path=self.task_path,
                raw_execution_dir=self.execution, task_id="TASK-001")

    def test_semantic_prepare_derives_active_identity_and_basis(self) -> None:
        self.task["steps"][0]["references"] = ["CMD-001"]
        preview = self.prepare_semantic()
        proposal = preview["proposal"]
        self.assertEqual(proposal["task_id"], "TASK-001")
        self.assertEqual(proposal["attempt_id"], "ATTEMPT-001")
        self.assertEqual(proposal["anchor_record_id"], "CMD-001")
        self.assertEqual(proposal["task_basis"], ["CMD-001", "STEP-001"])
        self.assertEqual(proposal["action"]["record_id"], "CMD-001")

    def test_semantic_prepare_builds_each_formal_action(self) -> None:
        cases = (
            ({"kind": "add_command", "command": {"mode": "argv", "argv": ["tool", "test"]}},
             {"kind": "add_command", "after_record_id": "CMD-001", "command": {"id": "CMD-002", "mode": "argv", "argv": ["tool", "test"]}}),
            ({"kind": "add_validation", "validation": {"kind": "automated", "command_positions": [1], "pass_condition": "Success.", "acceptance_positions": [1]}},
             {"kind": "add_validation", "validation": {"id": "VAL-001", "kind": "automated", "command_ids": ["CMD-001"], "pass_condition": "Success.", "acceptance_ids": ["ACCEPTANCE-001"]}}),
            ({"kind": "skip_record", "reason": "Cannot run."},
             {"kind": "skip_record", "record_id": "CMD-001", "reason": "Cannot run."}),
        )
        for semantic, formal in cases:
            with self.subTest(kind=semantic["kind"]):
                self.assertEqual(self.prepare_semantic(semantic)["proposal"]["action"], formal)
        self.task["operations"] = [{"id": "OP-001"}]
        self.task["validations"] = [{"id": "VAL-001"}]
        self.index["lock"]["record_id"] = "OP-001"
        operation = {"kind": "adjust_operation", "operation": {
            "kind": "file", "action": "Update.", "target": "src/app.py",
            "validation_position": 1, "command_position": 1}}
        self.assertEqual(self.prepare_semantic(operation)["proposal"]["action"], {
            "kind": "adjust_operation", "operation": {
                "id": "OP-001", "kind": "file", "action": "Update.", "target": "src/app.py",
                "validation_id": "VAL-001", "command_id": "CMD-001"}})

    def test_semantic_prepare_rejects_unresolvable_positions(self) -> None:
        action = {"kind": "add_validation", "validation": {
            "kind": "automated", "command_positions": [2], "pass_condition": "Success."}}
        with self.assertRaises(WorkError) as caught:
            self.prepare_semantic(action)
        self.assertEqual(caught.exception.code, "deviation_semantic_reference")

    def test_semantic_prepare_allocates_after_recorded_deviations(self) -> None:
        self.attempt["execution_deviations"] = [
            {"proposal": {"action": {"kind": "add_command", "command": {"id": "CMD-002"}}}},
            {"proposal": {"action": {"kind": "add_validation", "validation": {"id": "VAL-001"}}}},
        ]
        command = {"kind": "add_command", "command": {"mode": "argv", "argv": ["tool"]}}
        validation = {"kind": "add_validation", "validation": {
            "kind": "manual", "confirmer": "user", "criteria": "Reviewed."}}
        self.assertEqual(self.prepare_semantic(command)["proposal"]["action"]["command"]["id"], "CMD-003")
        self.assertEqual(self.prepare_semantic(validation)["proposal"]["action"]["validation"]["id"], "VAL-002")

    def test_semantic_prepare_rejects_stale_lock(self) -> None:
        self.index["lock"]["task_id"] = "TASK-002"
        with self.assertRaises(WorkError) as caught:
            self.prepare_semantic()
        self.assertEqual(caught.exception.code, "deviation_active_record")

    def test_preview_is_read_only_and_requires_semantic_review(self) -> None:
        before = self.snapshot()
        preview = self.prepare()
        self.assertEqual(before, self.snapshot())
        self.assertEqual(preview["action_validation"], "passed")
        self.assertEqual(preview["semantic_review"], "required")
        self.assertEqual(preview["classification"], "task_only")
        self.assertFalse(preview["blocking"])
        self.assertEqual(len(preview["preview_sha256"]), 64)

    def test_semantic_boundary_change_is_blocking_plan_and_task(self) -> None:
        self.proposal["impact"]["scope_changed"] = True
        preview = self.prepare()
        self.assertEqual(preview["classification"], "plan_and_task")
        self.assertTrue(preview["blocking"])

    def test_rejects_active_record_and_action_mismatches(self) -> None:
        self.index["lock"]["record_id"] = "CMD-002"
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "deviation_active_record")
        self.index["lock"]["record_id"] = "CMD-001"
        self.proposal["action"]["record_id"] = "CMD-002"
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "invalid_contract_value")

    def test_rejects_source_drift_and_fingerprint_tracks_proposal(self) -> None:
        with self.assertRaises(WorkError) as caught:
            self.prepare({self.task_path: b"stale"})
        self.assertEqual(caught.exception.code, "deviation_source_changed")
        first = self.prepare()["preview_sha256"]
        self.proposal["gap"] = "A different reviewed gap."
        self.assertNotEqual(first, self.prepare()["preview_sha256"])

    def test_records_approved_preview_and_rejects_duplicate(self) -> None:
        authorization = minimal_authorization()
        authorization["allowed_deviations"] = [copy.deepcopy(self.proposal["action"])]
        full_attempt = {
            "schema": "work-attempt/v1", "attempt_id": "ATTEMPT-001",
            "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001", "skill_id": None,
            "status": "in_progress", "task_collection_sha256": "1" * 64,
            "task_index_sha256": "2" * 64, "task_item_sha256": "3" * 64,
            "task_instructions_sha256": "4" * 64,
            "execute_instructions_sha256": "a" * 64,
            "hierarchy_selection_sha256": "5" * 64,
            "execute_skill_selection_sha256": "6" * 64,
            "authorization": authorization,
            "authorization_sha256": authorization_sha256(authorization),
            "started_at": "2026-09-01T10:00+08:00", "records": [],
        }
        from worklib.services.attempt import render_attempt_contract
        (self.root / self.attempt_path).write_bytes(
            render_attempt_contract(full_attempt, project_root=self.root)
        )
        preview = self.prepare()
        context = {
            "contract": self.contract, "validation": self.validation,
            "sources": {self.task_path: b"task"},
        }
        with patch("worklib.orchestration.execution.ExecutionOperations.load_task_execution_context", return_value=context), \
             patch("worklib.business_services.execution.deviation._validate_index_bytes", return_value=self.index), \
             patch("worklib.business_services.execution.deviation._validate_attempt_bytes", side_effect=lambda raw, **_: json.loads(raw)), \
             patch("worklib.business_services.execution.deviation.validate_execution_identity", return_value=self.row), \
             patch("worklib.business_services.execution.deviation.validate_execute_instructions"):
            before = self.snapshot()
            with self.assertRaises(WorkError) as reused:
                record_execution_deviation(
                    json.dumps(self.proposal).encode(), source="test",
                    approved_sha256=preview["preview_sha256"],
                    authorization_evidence=authorization["authorization_evidence"],
                    project_root=self.root,
                    user_config_root=str(self.root), raw_task_path=self.task_path,
                    raw_execution_dir=self.execution, task_id="TASK-001",
                )
            self.assertEqual(
                reused.exception.code, "deviation_record_authorization_evidence_reused"
            )
            self.assertEqual(before, self.snapshot())
            result = record_execution_deviation(
                json.dumps(self.proposal).encode(), source="test",
                approved_sha256=preview["preview_sha256"],
                authorization_evidence="User approved this exact runtime deviation.",
                project_root=self.root,
                user_config_root=str(self.root), raw_task_path=self.task_path,
                raw_execution_dir=self.execution, task_id="TASK-001",
            )
            self.assertEqual(result["deviation_id"], "DEVIATION-001")
            self.assertFalse(result["blocking"])
            with self.assertRaises(WorkError) as caught:
                record_execution_deviation(
                    json.dumps(self.proposal).encode(), source="test",
                    approved_sha256=preview["preview_sha256"],
                    authorization_evidence="User approved this exact runtime deviation.",
                    project_root=self.root,
                    user_config_root=str(self.root), raw_task_path=self.task_path,
                    raw_execution_dir=self.execution, task_id="TASK-001",
                )
        self.assertEqual(caught.exception.code, "deviation_record_duplicate")

        stored = json.loads((self.root / self.attempt_path).read_bytes())
        recorded = stored["execution_deviations"][0]
        self.assertEqual(
            recorded["supplemental_authorization"]["preview_sha256"],
            preview["preview_sha256"],
        )
        self.assertEqual(
            recorded["decision"]["evidence"],
            "User approved this exact runtime deviation.",
        )
