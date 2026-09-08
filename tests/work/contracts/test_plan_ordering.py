from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.plan_ordering import order_plan_contract


class PlanOrderingTests(unittest.TestCase):
    def test_orders_top_level_artifacts_and_plan_items(self) -> None:
        ordered = order_plan_contract(
            {
                "zzz": 2,
                "goals": [{"statement": "Goal", "id": "GOAL-001"}],
                "artifacts": {
                    "execution": "execution/example",
                    "task": "tasks/example/task.md",
                    "plan": "plans/example.md",
                },
                "schema": "work-plan/v1",
                "aaa": 1,
            }
        )

        self.assertEqual(
            list(ordered),
            ["schema", "artifacts", "goals", "aaa", "zzz"],
        )
        self.assertEqual(
            list(ordered["artifacts"]),
            ["plan", "task", "execution"],
        )
        self.assertEqual(list(ordered["goals"][0]), ["id", "statement"])

    def test_orders_instruction_skill_and_change_details(self) -> None:
        ordered = order_plan_contract(
            {
                "changes": [
                    {
                        "affected_ids": ["GOAL-001"],
                        "reason": "Update.",
                        "after": "new",
                        "before": "old",
                        "location": "goals[0]",
                        "date": "2026-09-07",
                        "id": "PLAN-CHANGE-001",
                    }
                ],
                "skill_selection": {
                    "selection_sha256": "a" * 64,
                    "skills": [
                        {
                            "bundle_sha256": "b" * 64,
                            "name": "frontend",
                            "id": "skill-1",
                        }
                    ],
                    "decision": "selected",
                    "schema": "work-skill-selection/v1",
                },
                "work_instruction_selection": {
                    "instructions_sha256": "c" * 64,
                    "references": [],
                    "sources": [
                        {
                            "canonical_sha256": "d" * 64,
                            "logical_name": "general",
                            "kind": "instruction",
                        }
                    ],
                    "resolved_paths": ["general"],
                    "selected_paths": [],
                },
            }
        )

        instruction_selection = ordered["work_instruction_selection"]
        self.assertEqual(
            list(instruction_selection),
            [
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256",
            ],
        )
        self.assertEqual(
            list(instruction_selection["sources"][0]),
            ["kind", "logical_name", "canonical_sha256"],
        )
        skill_selection = ordered["skill_selection"]
        self.assertEqual(
            list(skill_selection),
            ["schema", "decision", "skills", "selection_sha256"],
        )
        self.assertEqual(
            list(skill_selection["skills"][0]),
            ["id", "name", "bundle_sha256"],
        )
        self.assertEqual(
            list(ordered["changes"][0]),
            ["id", "date", "location", "before", "after", "reason", "affected_ids"],
        )


if __name__ == "__main__":
    unittest.main()
