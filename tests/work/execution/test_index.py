from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)


class ExecutionInstructionIndexTests(unittest.TestCase):
    def index(self) -> dict[str, object]:
        return build_initial_execution_index(
            {
                "requirement_id": "example",
                "spec_id": "TASK-SPEC-001",
                "tasks": [{"id": "TASK-001", "skill_id": None}],
            },
            {
                "task_sha256": "a" * 64,
                "instructions_sha256": "b" * 64,
                "task_instructions_sha256": {"TASK-001": "c" * 64},
                "hierarchy_selection_sha256": "f" * 64,
                "skill_selection_sha256": "d" * 64,
                "task_skill_ids": {"TASK-001": None},
            },
        )

    def validate(self, index: dict[str, object]) -> dict[str, object]:
        return validate_execution_index(
            render_execution_index(index),
            source="test",
            expected=index,
        )

    def test_initial_index_uses_instruction_fingerprints(self) -> None:
        index = self.index()
        result = self.validate(index)

        self.assertEqual(index["task_instructions_sha256"], "b" * 64)
        self.assertEqual(index["hierarchy_selection_sha256"], "f" * 64)
        self.assertEqual(index["skill_selection_sha256"], "d" * 64)
        self.assertIsNone(index["tasks"][0]["skill_id"])  # type: ignore[index]
        self.assertEqual(index["tasks"][0]["instructions_sha256"], "c" * 64)  # type: ignore[index]
        self.assertNotIn("task_rules_sha256", index)
        self.assertNotIn("rules_sha256", index["tasks"][0])  # type: ignore[index]
        self.assertEqual(result["overall_status"], "pending")

    def test_render_orders_instruction_fields_canonically(self) -> None:
        raw = render_execution_index(dict(reversed(self.index().items())))
        payload = json.loads(
            raw.decode("utf-8").split("```json\n", 1)[1].rsplit("\n```", 1)[0]
        )

        self.assertEqual(list(payload)[5], "task_instructions_sha256")
        self.assertEqual(list(payload)[6], "hierarchy_selection_sha256")
        self.assertEqual(list(payload)[7], "skill_selection_sha256")
        self.assertEqual(
            list(payload["tasks"][0]),
            ["id", "status", "skill_id", "instructions_sha256"],
        )

    def test_rejects_missing_skill_identity(self) -> None:
        index = copy.deepcopy(self.index())
        index.pop("skill_selection_sha256")

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["skill_selection_sha256"])

    def test_rejects_legacy_task_rules_fingerprint(self) -> None:
        index = copy.deepcopy(self.index())
        index["task_rules_sha256"] = index.pop("task_instructions_sha256")

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["task_instructions_sha256"],
        )
        self.assertEqual(
            context.exception.details["unknown"],
            ["task_rules_sha256"],
        )

    def test_rejects_legacy_per_task_rules_fingerprint(self) -> None:
        index = copy.deepcopy(self.index())
        task = index["tasks"][0]  # type: ignore[index]
        task["rules_sha256"] = task.pop("instructions_sha256")

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["instructions_sha256"])
        self.assertEqual(context.exception.details["unknown"], ["rules_sha256"])


if __name__ == "__main__":
    unittest.main()
