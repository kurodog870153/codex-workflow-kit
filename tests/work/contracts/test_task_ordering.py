from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.task_ordering import order_task_contract


class TaskOrderingTests(unittest.TestCase):
    def test_orders_top_level_fields_and_unknown_fields_stably(self) -> None:
        ordered = order_task_contract(
            {
                "zzz": 2,
                "readiness": {"spec_id": "TASK-SPEC-001", "status": "ready"},
                "tasks": [],
                "schema": "work-task/v1",
                "aaa": 1,
            }
        )

        self.assertEqual(
            list(ordered),
            ["schema", "tasks", "readiness", "aaa", "zzz"],
        )
        self.assertEqual(list(ordered["readiness"]), ["status", "spec_id"])

    def test_orders_nested_task_command_validation_and_change_fields(self) -> None:
        ordered = order_task_contract(
            {
                "changes": [
                    {
                        "edits": [
                            {
                                "after": "new",
                                "path": "tasks[0].goal",
                                "operation": "replace",
                                "before": "old",
                            }
                        ],
                        "reason": "Update goal.",
                        "id": "CHANGE-001",
                    }
                ],
                "tasks": [
                    {
                        "validations": [
                            {
                                "criteria": ["Passes."],
                                "id": "VAL-001",
                                "kind": "automated",
                                "command_ids": ["CMD-001"],
                            }
                        ],
                        "commands": [
                            {
                                "execution": {
                                    "shell": "pwsh",
                                    "os": "windows",
                                    "working_directory": ".",
                                },
                                "argv": ["tool"],
                                "mode": "argv",
                                "id": "CMD-001",
                            }
                        ],
                        "instruction_selection": {
                            "instructions_sha256": "a" * 64,
                            "sources": [
                                {
                                    "canonical_sha256": "b" * 64,
                                    "logical_name": "general",
                                    "kind": "instruction",
                                }
                            ],
                            "resolved_paths": ["general"],
                            "selected_paths": [],
                            "references": [],
                        },
                        "title": "Task",
                        "id": "TASK-001",
                    }
                ],
            }
        )

        task = ordered["tasks"][0]
        self.assertEqual(
            list(task),
            [
                "id",
                "title",
                "instruction_selection",
                "commands",
                "validations",
                "steps",
            ],
        )
        self.assertIsNone(task["steps"])
        selection = task["instruction_selection"]
        self.assertEqual(
            list(selection),
            [
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256",
            ],
        )
        self.assertEqual(
            list(selection["sources"][0]),
            ["kind", "logical_name", "canonical_sha256"],
        )
        command = task["commands"][0]
        self.assertEqual(list(command), ["id", "mode", "argv", "execution"])
        self.assertEqual(
            list(command["execution"]),
            ["working_directory", "os", "shell"],
        )
        self.assertEqual(
            list(task["validations"][0]),
            ["id", "kind", "command_ids", "criteria"],
        )
        change = ordered["changes"][0]
        self.assertEqual(list(change), ["id", "reason", "edits"])
        self.assertEqual(
            list(change["edits"][0]),
            ["operation", "path", "before", "after"],
        )


if __name__ == "__main__":
    unittest.main()
