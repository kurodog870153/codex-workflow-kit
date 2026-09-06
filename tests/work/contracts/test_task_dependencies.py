from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.task_dependencies import resolve_task_dependencies
from worklib.foundation.errors import WorkError


class TaskDependencyTests(unittest.TestCase):
    def test_resolves_stable_topological_order_and_ancestor_closure(self) -> None:
        order, ancestors = resolve_task_dependencies(
            ["TASK-001", "TASK-002", "TASK-003", "TASK-004"],
            {
                "TASK-001": [],
                "TASK-002": ["TASK-001"],
                "TASK-003": ["TASK-001"],
                "TASK-004": ["TASK-002", "TASK-003"],
            },
        )

        self.assertEqual(
            order,
            ["TASK-001", "TASK-002", "TASK-003", "TASK-004"],
        )
        self.assertEqual(ancestors["TASK-001"], set())
        self.assertEqual(ancestors["TASK-004"], {"TASK-001", "TASK-002", "TASK-003"})

    def test_preserves_input_order_for_tasks_ready_at_same_time(self) -> None:
        order, _ = resolve_task_dependencies(
            ["TASK-002", "TASK-001"],
            {"TASK-002": [], "TASK-001": []},
        )

        self.assertEqual(order, ["TASK-002", "TASK-001"])

    def test_rejects_dependency_cycle(self) -> None:
        with self.assertRaises(WorkError) as context:
            resolve_task_dependencies(
                ["TASK-001", "TASK-002"],
                {
                    "TASK-001": ["TASK-002"],
                    "TASK-002": ["TASK-001"],
                },
            )

        self.assertEqual(context.exception.code, "cyclic_task_dependency")

    def test_rejects_redundant_indirect_dependency(self) -> None:
        with self.assertRaises(WorkError) as context:
            resolve_task_dependencies(
                ["TASK-001", "TASK-002", "TASK-003"],
                {
                    "TASK-001": [],
                    "TASK-002": ["TASK-001"],
                    "TASK-003": ["TASK-001", "TASK-002"],
                },
            )

        self.assertEqual(context.exception.code, "indirect_task_dependency")
        self.assertEqual(context.exception.details["task_id"], "TASK-003")
        self.assertEqual(context.exception.details["dependency"], "TASK-001")


if __name__ == "__main__":
    unittest.main()
