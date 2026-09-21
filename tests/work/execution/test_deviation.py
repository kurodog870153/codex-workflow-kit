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

from worklib.models.execution import ExecutionDeviationProposalContract
from worklib.services.attempt import authorization_sha256, minimal_authorization
from worklib.workflows.execution import prepare_execution_deviation, record_execution_deviation
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
                     "commands": [{"id": "CMD-001"}], "operations": [], "validations": []}
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
        with patch("worklib.workflows.execution.ExecutionOperations.load_task_execution_context", return_value=context), \
             patch("worklib.business_services.execution.deviation._validate_index_bytes", return_value=self.index), \
             patch("worklib.business_services.execution.deviation._validate_attempt_bytes", return_value=self.attempt), \
             patch("worklib.business_services.execution.deviation.validate_execution_identity", return_value=self.row), \
             patch("worklib.business_services.execution.deviation.validate_execute_instructions"), \
             patch("worklib.business_services.execution.deviation.require_idle_writer"):
            return prepare_execution_deviation(json.dumps(self.proposal).encode(), source="test",
                project_root=self.root, user_config_root=str(self.root), raw_task_path=self.task_path,
                raw_execution_dir=self.execution, task_id="TASK-001")

    def test_preview_is_read_only_and_requires_semantic_review(self) -> None:
        before = self.snapshot()
        preview = self.prepare()
        self.assertEqual(before, self.snapshot())
        self.assertEqual(preview["action_validation"], "passed")
        self.assertEqual(preview["semantic_review"], "required")
        self.assertEqual(len(preview["preview_sha256"]), 64)

    def test_rejects_active_record_and_action_mismatches(self) -> None:
        self.index["lock"]["record_id"] = "CMD-002"
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "deviation_active_record")
        self.index["lock"]["record_id"] = "CMD-001"
        self.proposal["action"]["record_id"] = "CMD-002"
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "deviation_action_anchor")

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
        with patch("worklib.workflows.execution.ExecutionOperations.load_task_execution_context", return_value=context), \
             patch("worklib.business_services.execution.deviation._validate_index_bytes", return_value=self.index), \
             patch("worklib.business_services.execution.deviation._validate_attempt_bytes", side_effect=lambda raw, **_: json.loads(raw)), \
             patch("worklib.business_services.execution.deviation.validate_execution_identity", return_value=self.row), \
             patch("worklib.business_services.execution.deviation.validate_execute_instructions"):
            result = record_execution_deviation(
                json.dumps(self.proposal).encode(), source="test",
                approved_sha256=preview["preview_sha256"],
                project_root=self.root,
                user_config_root=str(self.root), raw_task_path=self.task_path,
                raw_execution_dir=self.execution, task_id="TASK-001",
            )
            self.assertEqual(result["deviation_id"], "DEVIATION-001")
            with self.assertRaises(WorkError) as caught:
                record_execution_deviation(
                    json.dumps(self.proposal).encode(), source="test",
                    approved_sha256=preview["preview_sha256"],
                    project_root=self.root,
                    user_config_root=str(self.root), raw_task_path=self.task_path,
                    raw_execution_dir=self.execution, task_id="TASK-001",
                )
        self.assertEqual(caught.exception.code, "deviation_record_duplicate")
