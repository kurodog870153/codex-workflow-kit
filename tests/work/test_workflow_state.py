from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "skills/work/scripts"))

from worklib.cli import main
from worklib.business_services.workflow.state import workflow_state
from worklib.services.workflow.state import inspect_requirement_state


SKILL_ROOT = Path(__file__).resolve().parents[2] / "skills" / "work"


class WorkflowStateTests(unittest.TestCase):
    def state(self, index, attempts):
        return workflow_state(
            SKILL_ROOT,
            "example",
            {"plan": "plan.json", "task": "task", "execution": "execution"},
            plan_validation={"plan_sha256": "a" * 64},
            draft=None,
            task_validation={"task_collection_sha256": "b" * 64},
            execution_index=index,
            latest_attempts=attempts,
        )

    def test_valid_active_execution_lock_continues_execution(self):
        fingerprint = "c" * 64
        index = {
            "overall_status": "in_progress",
            "lock": {"kind": "execution", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
                     "execute_instructions_sha256": fingerprint},
            "tasks": [{"id": "TASK-001", "status": "in_progress", "latest_attempt": "ATTEMPT-001"}],
        }
        attempts = {"TASK-001": {"status": "in_progress", "task_id": "TASK-001",
                    "attempt_id": "ATTEMPT-001", "execute_instructions_sha256": fingerprint}}

        result = self.state(index, attempts)

        self.assertEqual((result["status"], result["next_action"]),
                         ("execution_in_progress", "continue_execution"))
        self.assertFalse(result["requires_user_confirmation"])

    def test_mismatched_active_execution_lock_requires_recovery(self):
        index = {
            "overall_status": "in_progress",
            "lock": {"kind": "execution", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
                     "execute_instructions_sha256": "c" * 64},
            "tasks": [{"id": "TASK-001", "status": "in_progress", "latest_attempt": "ATTEMPT-001"}],
        }
        attempts = {"TASK-001": {"status": "in_progress", "task_id": "TASK-001",
                    "attempt_id": "ATTEMPT-001", "execute_instructions_sha256": "d" * 64}}

        result = self.state(index, attempts)

        self.assertEqual(result["next_action"], "inspect_recovery")

    def test_closed_attempt_pending_deviation_routes_to_reconciliation(self):
        index = {"overall_status": "completed", "tasks": [
            {"id": "TASK-001", "status": "completed", "latest_attempt": "ATTEMPT-001"}
        ]}
        attempts = {"TASK-001": {"status": "completed", "execution_deviations": [{
            "deviation_id": "DEVIATION-001", "decision": {"outcome": "approved"},
            "reconciliation_status": "pending",
        }]}}

        result = self.state(index, attempts)

        self.assertEqual((result["status"], result["next_action"]),
                         ("execution_reconciliation_pending", "review_reconciliation"))
        self.assertEqual(result["details"]["pending_deviation_ids"], ["DEVIATION-001"])

    def test_reconciliation_ledger_resolution_removes_pending_route(self):
        index = {"overall_status": "completed", "tasks": [
            {"id": "TASK-001", "status": "completed", "latest_attempt": "ATTEMPT-001"}
        ]}
        attempts = {"TASK-001": {
            "status": "completed",
            "_reconciliation_resolved_ids": ["DEVIATION-001"],
            "execution_deviations": [{
                "deviation_id": "DEVIATION-001",
                "decision": {"outcome": "approved"},
                "reconciliation_status": "pending",
            }],
        }}
        result = self.state(index, attempts)
        self.assertEqual(
            (result["status"], result["next_action"]),
            ("execution_completed", "review_completion"),
        )

    def test_missing_plan_returns_deterministic_confirmed_gate_without_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = io.StringIO()
            code = main(["--project-root", str(root), "workflow", "status",
                         "--requirement-id", "example", "--user-config-root", str(root)], stdout=output)
            result = json.loads(output.getvalue())["data"]
            self.assertEqual(code, 0)
            self.assertEqual((result["status"], result["next_action"]), ("plan_required", "prepare_plan"))
            self.assertTrue(result["requires_user_confirmation"])
            self.assertEqual(result["routing_status"], "VALID")
            self.assertEqual(result["required_instruction_sources"], [
                "work.instruction-loading",
                "work.shared.invocation",
                "work.shared.skill-discovery",
                "work.shared.skill-selection",
                "work.shared.source-loading",
                "work.shared.artifact-paths",
                "work.shared.fingerprints",
                "work.workflow.plan",
                "work.workflow.plan.apply-confirmed-skills",
                "work.workflow.plan.complete-the-request",
                "work.workflow.plan.use-the-deterministic-plan-contract",
            ])
            self.assertNotIn("work.workflow.task", result["required_instruction_sources"])
            self.assertNotIn("work.workflow.execute", result["required_instruction_sources"])
            self.assertEqual(result["source_order"], result["required_instruction_sources"])
            self.assertEqual(len(result["selection_sha256"]), 64)
            self.assertEqual(
                result["selection_manifest"]["selection_sha256"],
                result["selection_sha256"],
            )
            self.assertEqual(list(root.rglob("*")), [])

    def test_status_and_next_are_equivalent_read_only_views(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            values = []
            for command in ("status", "next"):
                output = io.StringIO()
                self.assertEqual(main(["--project-root", str(root), "workflow", command,
                    "--requirement-id", "example", "--user-config-root", str(root)], stdout=output), 0)
                values.append(json.loads(output.getvalue())["data"])
            self.assertEqual(values[0], values[1])

    def test_non_default_plan_path_drives_all_fsm_artifact_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = {
                "plan": "custom/plans/example.json",
                "task": "custom/tasks/example/index.json",
                "execution": "custom/executions/example",
            }
            plan = root / artifacts["plan"]
            plan.parent.mkdir(parents=True)
            plan.write_text(json.dumps({
                "schema": "work-plan/v1",
                "requirement_id": "example",
                "artifacts": artifacts,
            }), encoding="utf-8")

            selected, observed = inspect_requirement_state(
                root, "example", plan_path=artifacts["plan"],
            )

            self.assertEqual(selected, artifacts)
            self.assertEqual(
                observed["execution_index_path"],
                root / artifacts["execution"] / "index.json",
            )

    def test_non_default_plan_path_must_match_declared_plan_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = root / "custom/plans/example.json"
            plan.parent.mkdir(parents=True)
            plan.write_text(json.dumps({
                "schema": "work-plan/v1",
                "requirement_id": "example",
                "artifacts": {
                    "plan": "other/plans/example.json",
                    "task": "custom/tasks/example/index.json",
                    "execution": "custom/executions/example",
                },
            }), encoding="utf-8")

            with self.assertRaises(Exception):
                inspect_requirement_state(
                    root, "example", plan_path="custom/plans/example.json",
                )


if __name__ == "__main__":
    unittest.main()
