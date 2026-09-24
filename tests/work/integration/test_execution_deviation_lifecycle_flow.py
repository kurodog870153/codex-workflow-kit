from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
sys.path.insert(0, str(SKILL_ROOT / "scripts"))

from tests.work.contracts import test_task_collection
from worklib.business_services.instruction import build_instruction_selection
from worklib.business_services.task import load_task_collection
from worklib.business_services.workflow.state import load_latest_attempts, workflow_state
from worklib.orchestration.execution import (
    begin_record, close_attempt, finish_record,
    prepare_semantic_execution_deviation, record_execution_deviation, start_attempt,
)
from worklib.orchestration.task import (
    preview_specification_reconciliation, publish_specification_reconciliation,
)
from worklib.services.attempt import minimal_authorization
from worklib.services.attempt.validation import (
    build_initial_execution_index, render_execution_index,
)
from worklib.technical.infrastructure.json_contract import parse_json_contract


class ExecutionDeviationLifecycleFlowTests(unittest.TestCase):
    def test_runtime_skip_deviation_completes_and_reconciles(self) -> None:
        fixture = test_task_collection.TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        root = fixture.root
        validation = load_task_collection(root, str(root), fixture.index_path)
        contract = validation["collection_contract"]
        task = contract["tasks"][0]
        execution_dir = contract["artifacts"]["execution"]
        execution_path = root / execution_dir
        execution_path.mkdir(parents=True)
        index_path = execution_path / "index.json"
        index_path.write_bytes(render_execution_index(
            build_initial_execution_index(contract, validation)
        ))
        execute_selection = build_instruction_selection(
            skill_root=SKILL_ROOT, mode="execute", selected_paths=[],
            reference_names=["execute.general.execution-records"],
        )
        preflight = {
            "task_spec_id": contract["spec_id"], "task_id": "TASK-001",
            "skill_id": None,
            "task_collection_sha256": validation["task_collection_sha256"],
            "task_index_sha256": validation["task_index_sha256"],
            "task_item_sha256": validation["task_item_sha256"]["TASK-001"],
            "task_instructions_sha256": validation["task_instructions_sha256"]["TASK-001"],
            "execute_instructions_sha256": execute_selection["instructions_sha256"],
            "hierarchy_selection_sha256": validation["hierarchy_selection_sha256"],
            "execute_skill_selection": {"selection_sha256": validation["skill_selection_sha256"]},
            "execution_dir": execution_dir, "snapshot_sha256": "e" * 64,
        }
        common = {
            "project_root": root, "user_config_root": str(root),
            "raw_task_path": fixture.index_path, "raw_execution_dir": execution_dir,
            "task_id": "TASK-001",
        }
        authorization = minimal_authorization()
        authorization["validations"] = [task["validations"][0]]
        start_request = {
            "schema": "work-attempt-start-request/v1",
            "worktree_snapshot_sha256": preflight["snapshot_sha256"],
            "authorization": authorization,
        }
        with patch(
            "worklib.business_services.execution.attempt_start.inspect_execute_worktree",
            return_value=preflight,
        ):
            started = start_attempt(
                json.dumps(start_request).encode(), source="test", **common
            )
        self.assertEqual(started["attempt_id"], "ATTEMPT-001")

        reserved = begin_record(base_record_id="VAL-001", **common)
        proposal = {
            "schema": "work-execution-deviation-proposal/v1",
            "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
            "anchor_record_id": reserved["record_id"],
            "task_basis": ["VAL-001", *(
                step["id"] for step in task["steps"]
                if "VAL-001" in step.get("references", []))],
            "gap": "The runtime validation result requires an explicit retry.",
            "action": {
                "kind": "skip_record", "record_id": reserved["record_id"],
                "reason": "Retain the failed runtime evidence for the retry decision.",
            },
            "modifiable_files": [],
            "impact": {
                "summary": "Record the runtime-only retry decision.",
                "requirement_changed": False, "scope_changed": False,
                "acceptance_criteria_changed": False, "deliverables_changed": False,
                "safety_boundary_changed": False,
                "external_side_effect_boundary_changed": False,
            },
            "side_effects": [],
        }
        semantic = {key: proposal[key] for key in (
            "gap", "modifiable_files", "impact", "side_effects")}
        semantic["action"] = {
            "kind": "skip_record", "reason": proposal["action"]["reason"]}
        preview = prepare_semantic_execution_deviation(
            json.dumps(semantic).encode(), source="test", **common
        )
        self.assertEqual(preview["proposal"], proposal)
        proposal = preview["proposal"]
        recorded = record_execution_deviation(
            json.dumps(proposal).encode(), source="test",
            approved_sha256=preview["preview_sha256"],
            authorization_evidence="User approved the runtime deviation.",
            **common,
        )
        self.assertEqual(recorded["deviation_id"], "DEVIATION-001")
        finish_record(json.dumps({
            "schema": "work-record-finish-request/v1",
            "record": {
                "status": "skipped",
                "reason": proposal["action"]["reason"],
                "deviation_id": recorded["deviation_id"],
            },
        }).encode(), source="test", **common)
        close_attempt(json.dumps({
            "schema": "work-attempt-close-request/v1", "status": "completed",
        }).encode(), source="test", **common)

        attempt_path = root / started["attempt_path"]
        immutable_attempt = attempt_path.read_bytes()
        closed_attempt = parse_json_contract(immutable_attempt, source=str(attempt_path))
        deviation = closed_attempt["execution_deviations"][0]
        self.assertEqual(
            deviation["supplemental_authorization"]["preview_sha256"],
            deviation["approved_preview_sha256"],
        )
        self.assertEqual(closed_attempt["records"][0]["status"], "skipped")
        self.assertEqual(closed_attempt["status"], "completed")
        reconciliation_request = {
            "schema": "work-spec-reconciliation-preview-request/v1",
            "attempt_path": started["attempt_path"], "choice": "retain_only",
            "deviation_ids": [], "migration": None,
        }
        reconciliation = preview_specification_reconciliation(
            json.dumps(reconciliation_request).encode(), project_root=root,
            user_config_root=str(root),
        )
        publication = publish_specification_reconciliation(
            json.dumps(reconciliation_request).encode(),
            approved_sha256=reconciliation["fingerprint"], project_root=root,
            user_config_root=str(root),
        )
        self.assertEqual(publication["retained_deviation_ids"], ["DEVIATION-001"])
        self.assertEqual(attempt_path.read_bytes(), immutable_attempt)

        index = parse_json_contract(index_path.read_bytes(), source=str(index_path))
        attempts = load_latest_attempts(root, contract["artifacts"], index)
        state = workflow_state(
            SKILL_ROOT, "example", contract["artifacts"],
            plan_validation={"plan_sha256": "a" * 64}, draft=None,
            task_validation={"task_collection_sha256": validation["task_collection_sha256"]},
            execution_index=index, latest_attempts=attempts,
        )
        self.assertNotEqual(state["next_action"], "review_reconciliation")
        self.assertEqual(
            parse_json_contract(index_path.read_bytes(), source=str(index_path))["overall_status"],
            "completed",
        )


if __name__ == "__main__":
    unittest.main()
