from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
SCRIPT_ROOT = SKILL_ROOT / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.execution.preflight import execute_preflight
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
)
from worklib.instructions.selection import build_instruction_selection
from worklib.contracts.plan import render_plan_contract, validate_plan_file
from worklib.skills.catalog import SkillRoot, snapshot_catalog_skill
from worklib.skills.selection import selection_sha256
from worklib.contracts.task import render_task_contract, validate_task_contract
from worklib.instructions.task_selection import (
    build_task_document_instruction_selection,
)
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.hierarchy.selection import build_hierarchy_selection


class ExecuteInstructionPreflightTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.project_root = Path(self.temporary_directory.name).resolve()
        (self.project_root / "src.txt").write_text("source\n", encoding="utf-8")
        self.artifacts = {
            "plan": "outputs/work/plans/example.md",
            "task": "outputs/work/tasks/example.md",
            "execution": "outputs/work/executions/example",
        }
        skill_root_path = self.project_root / ".agents" / "skills"
        skill_path = skill_root_path / "backend"
        skill_path.mkdir(parents=True)
        (skill_path / "SKILL.md").write_text(
            """---
name: backend
description: Build backends.
metadata:
  work-modes: plan,task,execute
---
Instructions
""",
            encoding="utf-8",
        )
        self.skill_root = SkillRoot("repo", ".agents/skills", skill_root_path)
        snapshot = snapshot_catalog_skill(self.skill_root, "backend/SKILL.md")
        summary = snapshot["skill"]
        bundle = snapshot["bundle"]
        assert isinstance(summary, dict) and isinstance(bundle, dict)
        self.selected_skill = {
            "id": summary["id"],
            "name": summary["name"],
            "scope": summary["scope"],
            "root": summary["root"],
            "source": summary["source"],
            "description": summary["description"],
            "mode_support": {
                "plan": "declared", "task": "declared", "execute": "declared"
            },
            "allow_implicit_invocation": summary["allow_implicit_invocation"],
            "dependency_status": "available",
            "summary_sha256": summary["summary_sha256"],
            "bundle_sha256": bundle["bundle_sha256"],
            "recommendation_reason": "The task requires backend work.",
        }

        hierarchy_selection = build_hierarchy_selection(
            {
                "decision": "instruction_paths",
                "selections": [
                    {
                        "path": "web/backend/java/jpa",
                        "recommendation_reason": "The work uses JPA persistence.",
                    }
                ],
            },
            skill_root=SKILL_ROOT,
        )
        plan = {
            "schema": "work-plan/v1",
            "requirement_id": "example",
            "status": "confirmed",
            "title": "Example Plan",
            "summary": "Plan summary.",
            "artifacts": self.artifacts,
            "hierarchy_selection": hierarchy_selection,
            "work_instruction_selection": build_work_instruction_selection(
                skill_root=SKILL_ROOT,
                mode="plan",
                selected_paths=hierarchy_selection["selected_paths"],
            ),
            "skill_selection": {
                "schema": "work-skill-selection/v1", "decision": "external_skills",
                "skills": [self.selected_skill],
                "selection_sha256": selection_sha256("external_skills", [self.selected_skill]),
            },
            "goals": [{"id": "GOAL-001", "statement": "Complete the goal."}],
            "scope": [
                {
                    "id": "SCOPE-001",
                    "kind": "in_scope",
                    "statement": "Handle the scope.",
                    "goal_ids": ["GOAL-001"],
                }
            ],
            "deliverables": [
                {
                    "id": "DELIVERABLE-001",
                    "statement": "Produce the result.",
                    "goal_ids": ["GOAL-001"],
                    "acceptance_ids": ["ACCEPTANCE-001"],
                }
            ],
            "acceptance_criteria": [
                {
                    "id": "ACCEPTANCE-001",
                    "statement": "The result is observable.",
                    "deliverable_ids": ["DELIVERABLE-001"],
                }
            ],
        }
        plan_path = self.project_root / self.artifacts["plan"]
        plan_path.parent.mkdir(parents=True)
        plan_path.write_bytes(render_plan_contract(plan))
        plan_validation = validate_plan_file(
            self.project_root,
            str(self.project_root),
            self.artifacts["plan"],
            skill_roots=[self.skill_root],
        )

        task_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="task",
            selected_paths=["web/backend/java/jpa"],
            reference_names=["task.general.task-records"],
        )
        self.task_contract: dict[str, object] = {
            "schema": "work-task/v1",
            "requirement_id": "example",
            "spec_id": "TASK-SPEC-001",
            "status": "confirmed",
            "title": "Example TASK",
            "summary": "Task summary.",
            "artifacts": self.artifacts,
            "source_plan": {
                "canonical_sha256": plan_validation["plan_sha256"],
                "hierarchy_selection_sha256": plan_validation[
                    "hierarchy_selection_sha256"
                ],
            },
            "instruction_selection": build_task_document_instruction_selection(
                [task_selection],
                skill_root=SKILL_ROOT,
            ),
            "tasks": [
                {
                    "id": "TASK-001",
                    "title": "Modify source",
                    "skill_id": self.selected_skill["id"],
                    "instruction_selection": task_selection,
                    "traceability": {
                        "goal_ids": ["GOAL-001"],
                        "deliverable_ids": ["DELIVERABLE-001"],
                        "acceptance_ids": ["ACCEPTANCE-001"],
                    },
                    "goal": "Modify and validate the source.",
                    "files": [
                        {
                            "id": "FILE-001",
                            "action": "modify",
                            "path": "src.txt",
                        }
                    ],
                    "steps": [
                        {
                            "id": "STEP-001",
                            "action": "Modify the source.",
                            "references": ["FILE-001", "VAL-001"],
                        }
                    ],
                    "validations": [
                        {
                            "id": "VAL-001",
                            "kind": "manual",
                            "confirmer": "User",
                            "criteria": "The source is correct.",
                            "acceptance_ids": ["ACCEPTANCE-001"],
                        }
                    ],
                }
            ],
            "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
        }
        task_raw = render_task_contract(self.task_contract)
        task_path = self.project_root / self.artifacts["task"]
        task_path.parent.mkdir(parents=True)
        task_path.write_bytes(task_raw)
        task_validation = validate_task_contract(
            task_raw,
            source=str(task_path),
            actual_task_path=self.artifacts["task"],
            project_root=self.project_root,
            user_config_root=str(self.project_root),
            skill_roots=[self.skill_root],
        )

        self.index = build_initial_execution_index(
            self.task_contract,
            task_validation,
        )
        execution_path = self.project_root / self.artifacts["execution"]
        execution_path.mkdir(parents=True)
        self.index_path = execution_path / "index.md"
        self.write_index(self.index)

    def write_index(self, index: dict[str, object]) -> None:
        self.index_path.write_bytes(render_execution_index(index))

    def preflight(self) -> dict[str, object]:
        return execute_preflight(
            project_root=self.project_root,
            user_config_root=str(self.project_root),
            raw_task_path=self.artifacts["task"],
            raw_execution_dir=self.artifacts["execution"],
            task_id="TASK-001",
            skill_roots=[self.skill_root],
        )

    def test_returns_execute_instruction_selection(self) -> None:
        result = self.preflight()
        selection = result["execute_instruction_selection"]

        self.assertEqual(result["eligibility"], "passed")
        self.assertEqual(
            result["hierarchy_selection_sha256"],
            self.index["hierarchy_selection_sha256"],
        )
        self.assertEqual(
            selection["selected_paths"],  # type: ignore[index]
            ["web/backend/java/jpa"],
        )
        self.assertEqual(
            selection["references"],  # type: ignore[index]
            ["execute.general.execution-records"],
        )
        self.assertEqual(
            result["execute_instructions_sha256"],
            selection["instructions_sha256"],  # type: ignore[index]
        )
        self.assertNotIn("task_rules_sha256", result)
        self.assertNotIn("execute_rules_sha256", result)
        self.assertNotIn("execute_rule_selection", result)
        skill_selection = result["execute_skill_selection"]
        self.assertEqual(skill_selection["decision"], "external_skills")  # type: ignore[index]
        self.assertEqual(skill_selection["skills"], [self.selected_skill])  # type: ignore[index]

    def test_rejects_selected_skill_drift(self) -> None:
        skill_file = self.skill_root.path / "backend" / "SKILL.md"
        skill_file.write_text(
            skill_file.read_text(encoding="utf-8") + "Changed\n",
            encoding="utf-8",
        )

        with self.assertRaises(WorkError) as context:
            self.preflight()

        self.assertEqual(context.exception.code, "selected_skill_snapshot_mismatch")

    def test_rejects_stale_index_document_instruction_fingerprint(self) -> None:
        index = copy.deepcopy(self.index)
        index["task_instructions_sha256"] = "0" * 64
        self.write_index(index)

        with self.assertRaises(WorkError) as context:
            self.preflight()

        self.assertEqual(
            context.exception.code,
            "execute_preflight_index_identity_mismatch",
        )

    def test_rejects_stale_index_task_instruction_fingerprint(self) -> None:
        index = copy.deepcopy(self.index)
        index["tasks"][0]["instructions_sha256"] = "0" * 64  # type: ignore[index]
        self.write_index(index)

        with self.assertRaises(WorkError) as context:
            self.preflight()

        self.assertEqual(
            context.exception.code,
            "execute_preflight_task_instructions_mismatch",
        )

    def test_pending_retry_loads_recovery_instruction_reference(self) -> None:
        index = copy.deepcopy(self.index)
        task = index["tasks"][0]  # type: ignore[index]
        task["status"] = "pending_retry"
        task["status_reason"] = {"kind": "attempt", "ref": "ATTEMPT-001"}
        self.write_index(index)

        result = self.preflight()

        self.assertEqual(
            result["execute_instruction_selection"]["references"],  # type: ignore[index]
            [
                "execute.general.execution-records",
                "execute.general.execution-recovery",
            ],
        )


if __name__ == "__main__":
    unittest.main()
