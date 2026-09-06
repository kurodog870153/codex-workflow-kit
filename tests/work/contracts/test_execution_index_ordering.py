from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.execution_index_ordering import order_execution_index


class ExecutionIndexOrderingTests(unittest.TestCase):
    def test_orders_top_level_and_task_status_reason(self) -> None:
        ordered = order_execution_index(
            {
                "zzz": 2,
                "tasks": [
                    {
                        "status_reason": {"ref": "ATTEMPT-001", "kind": "attempt"},
                        "latest_attempt": "ATTEMPT-001",
                        "status": "in_progress",
                        "id": "TASK-001",
                    }
                ],
                "overall_status": "in_progress",
                "schema": "work-execution-index/v1",
                "aaa": 1,
            }
        )

        self.assertEqual(
            list(ordered),
            ["schema", "overall_status", "tasks", "aaa", "zzz"],
        )
        task = ordered["tasks"][0]
        self.assertEqual(
            list(task),
            ["id", "status", "latest_attempt", "status_reason"],
        )
        self.assertEqual(list(task["status_reason"]), ["kind", "ref"])

    def test_orders_lock_and_canonicalizes_command_correction(self) -> None:
        ordered = order_execution_index(
            {
                "lock": {
                    "execute_instructions_sha256": "a" * 64,
                    "command_correction": {
                        "authorization_evidence": "Approved.",
                        "reason": "Use new argument.",
                        "actual_command": {
                            "argv": ["tool", "new"],
                            "mode": "argv",
                        },
                        "original_command": {
                            "argv": ["tool", "old"],
                            "mode": "argv",
                        },
                    },
                    "record_id": "CMD-001",
                    "attempt_id": "ATTEMPT-001",
                    "task_id": "TASK-001",
                    "kind": "execution",
                }
            }
        )

        lock = ordered["lock"]
        self.assertEqual(
            list(lock),
            [
                "kind",
                "task_id",
                "attempt_id",
                "record_id",
                "command_correction",
                "execute_instructions_sha256",
            ],
        )
        correction = lock["command_correction"]
        self.assertEqual(
            list(correction),
            [
                "original_command",
                "actual_command",
                "reason",
                "authorization_evidence",
            ],
        )
        self.assertEqual(
            list(correction["original_command"]),
            ["mode", "argv"],
        )


if __name__ == "__main__":
    unittest.main()
