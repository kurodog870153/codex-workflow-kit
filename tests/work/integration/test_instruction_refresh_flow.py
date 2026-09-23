from __future__ import annotations

import shutil
import sys
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_task_collection as fixtures
from worklib.business_services.instruction.refresh import (
    apply_source_refresh,
    apply_source_refresh_all,
    preview_source_refresh,
    preview_source_refresh_all,
)
from worklib.business_services.task import load_task_collection
from worklib.business_services.plan import render_plan_contract
from worklib.models.common.errors import WorkError
from worklib.services.attempt import build_initial_execution_index, render_execution_index
from worklib.technical.infrastructure.json_contract import parse_json_contract


class InstructionRefreshFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture = fixtures.TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.root = fixture.root
        self.skill_root = self.root / "work-skill"
        shutil.copytree(PROJECT_ROOT / "skills" / "work", self.skill_root)

        validation = load_task_collection(
            self.root, str(self.root), fixture.index_path
        )
        self.artifacts = validation["collection_contract"]["artifacts"]
        execution_path = self.root / self.artifacts["execution"] / "index.json"
        execution_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.write_bytes(
            render_execution_index(
                build_initial_execution_index(
                    validation["collection_contract"], validation
                )
            )
        )
        self.execution_path = execution_path

    def drift_sources(self) -> None:
        for mode in ("plan", "task"):
            path = self.skill_root / "references" / "workflows" / f"{mode}.md"
            path.write_bytes(path.read_bytes() + b"\nCompatible wording update.\n")

    def test_preview_and_apply_refresh_complete_dependency_chain(self) -> None:
        history = (
            self.execution_path.parent
            / "TASK-001"
            / "ATTEMPT-001"
            / "attempt.json"
        )
        history.parent.mkdir(parents=True)
        history.write_bytes(b"historical evidence\n")
        before_history = history.read_bytes()
        self.drift_sources()

        preview = preview_source_refresh(self.root, self.skill_root, "example")

        self.assertEqual(preview["status"], "refreshable")
        self.assertEqual(
            preview["affected"],
            {"plans": 1, "task_items": 1, "task_indexes": 1, "execution_indexes": 1},
        )
        result = apply_source_refresh(
            self.root,
            self.skill_root,
            "example",
            preview["approved_sha256"],
        )
        self.assertEqual(result["status"], "updated")
        repeated = apply_source_refresh(
            self.root, self.skill_root, "example", preview["approved_sha256"],
        )
        self.assertEqual(repeated["status"], "already_completed")
        self.assertEqual(history.read_bytes(), before_history)
        self.assertTrue((self.root / result["completion_marker"]).is_file())
        refreshed_plan = parse_json_contract(
            (self.root / self.artifacts["plan"]).read_bytes(), source="refreshed plan",
        )
        self.assertIn("routing_manifest", refreshed_plan["work_instruction_selection"])
        refreshed_execution = parse_json_contract(
            self.execution_path.read_bytes(), source="refreshed execution",
        )
        self.assertEqual(
            refreshed_execution["instruction_selection_manifest"]["router_compatibility_revision"], 3,
        )
        self.assertEqual(
            preview_source_refresh(self.root, self.skill_root, "example")["status"],
            "valid",
        )

    def test_batch_preview_and_apply_use_one_approved_set(self) -> None:
        self.drift_sources()
        preview = preview_source_refresh_all(self.root, self.skill_root)
        self.assertEqual(preview["status"], "refreshable")
        result = apply_source_refresh_all(self.root, self.skill_root, preview["approved_sha256"])
        self.assertEqual(result["status"], "updated")
        self.assertEqual(result["semantics"], "recoverable_sequential")
        self.assertEqual(result["completed_requirement_ids"], ["example"])
        self.assertEqual([row["requirement_id"] for row in result["publications"]], ["example"])
        self.assertEqual(
            apply_source_refresh_all(
                self.root, self.skill_root, preview["approved_sha256"]
            )["status"],
            "already_completed",
        )

    def test_active_attempt_binding_blocks_refresh(self) -> None:
        execution = parse_json_contract(
            self.execution_path.read_bytes(), source="execution index"
        )
        execution["tasks"][0]["status"] = "in_progress"
        execution["tasks"][0]["latest_attempt"] = "ATTEMPT-001"
        execution["overall_status"] = "in_progress"
        self.execution_path.write_bytes(render_execution_index(execution))
        self.drift_sources()

        preview = preview_source_refresh(self.root, self.skill_root, "example")

        self.assertEqual(preview["status"], "review_required")
        self.assertEqual(
            preview["blocked"],
            [{"path": self.artifacts["execution"] + "/index.json", "reason": "active_attempt_snapshot"}],
        )
        self.assertEqual(preview["files"], [])

    def test_apply_rejects_source_drift_after_preview(self) -> None:
        self.drift_sources()
        preview = preview_source_refresh(self.root, self.skill_root, "example")
        workflow = self.skill_root / "references" / "workflows" / "plan.md"
        workflow.write_bytes(workflow.read_bytes() + b"Additional drift.\n")

        with self.assertRaises(WorkError) as caught:
            apply_source_refresh(
                self.root, self.skill_root, "example", preview["approved_sha256"]
            )

        self.assertEqual(caught.exception.code, "source_refresh_approval_changed")

    def test_apply_rejects_output_drift_after_preview(self) -> None:
        self.drift_sources()
        preview = preview_source_refresh(self.root, self.skill_root, "example")
        plan_path = self.root / self.artifacts["plan"]
        plan = parse_json_contract(plan_path.read_bytes(), source="plan")
        plan["summary"] += " External edit."
        plan_path.write_bytes(render_plan_contract(plan))

        with self.assertRaises(WorkError) as caught:
            apply_source_refresh(
                self.root, self.skill_root, "example", preview["approved_sha256"]
            )

        self.assertEqual(caught.exception.code, "source_refresh_approval_changed")

    def test_non_default_execution_path_is_preserved_for_transaction(self) -> None:
        custom_execution = "outputs/work/custom/runtime"
        plan_path = self.root / self.artifacts["plan"]
        plan = parse_json_contract(plan_path.read_bytes(), source="plan")
        plan["artifacts"]["execution"] = custom_execution
        plan_path.write_bytes(render_plan_contract(plan))
        custom_path = self.root / custom_execution / "index.json"
        custom_path.parent.mkdir(parents=True)
        self.execution_path.replace(custom_path)
        self.execution_path = custom_path
        self.drift_sources()

        preview = preview_source_refresh(self.root, self.skill_root, "example")
        result = apply_source_refresh(
            self.root, self.skill_root, "example", preview["approved_sha256"]
        )

        self.assertTrue(result["journal"].startswith(custom_execution + "/"))
        self.assertTrue((self.root / result["completion_marker"]).is_file())
        self.assertEqual(
            apply_source_refresh(
                self.root, self.skill_root, "example", preview["approved_sha256"]
            )["status"],
            "already_completed",
        )

    def test_plan_only_requirement_refreshes_without_task_or_execution(self) -> None:
        source_plan_path = self.root / self.artifacts["plan"]
        plan = parse_json_contract(source_plan_path.read_bytes(), source="plan")
        plan["requirement_id"] = "planonly"
        plan["artifacts"] = {
            "plan": "outputs/work/plans/planonly.json",
            "task": "outputs/work/tasks/planonly/index.json",
            "execution": "outputs/work/executions/planonly",
        }
        plan_path = self.root / plan["artifacts"]["plan"]
        plan_path.write_bytes(render_plan_contract(plan))
        before = plan_path.read_bytes()
        plan_workflow = self.skill_root / "references" / "workflows" / "plan.md"
        plan_workflow.write_bytes(
            plan_workflow.read_bytes() + b"\nCompatible Plan-only update.\n"
        )

        preview = preview_source_refresh(self.root, self.skill_root, "planonly")
        self.assertEqual(
            preview["affected"],
            {"plans": 1, "task_items": 0, "task_indexes": 0, "execution_indexes": 0},
        )
        result = apply_source_refresh(
            self.root, self.skill_root, "planonly", preview["approved_sha256"]
        )
        self.assertEqual(result["updated_files"], [plan["artifacts"]["plan"]])
        self.assertNotEqual(plan_path.read_bytes(), before)
        self.assertTrue((self.root / result["completion_marker"]).is_file())


if __name__ == "__main__":
    unittest.main()
