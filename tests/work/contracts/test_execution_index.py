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
                "schema": "work-task-collection-validation/v1",
                "task_collection_sha256": "a" * 64,
                "task_index_sha256": "1" * 64,
                "task_item_sha256": {"TASK-001": "2" * 64},
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
            raw.decode("utf-8")
        )

        self.assertEqual(list(payload)[4], "task_collection_sha256")
        self.assertEqual(list(payload)[5], "task_index_sha256")
        self.assertEqual(list(payload)[6], "task_instructions_sha256")
        self.assertEqual(list(payload)[7], "hierarchy_selection_sha256")
        self.assertEqual(list(payload)[8], "skill_selection_sha256")
        self.assertEqual(
            list(payload["tasks"][0]),
            [
                "id",
                "status",
                "skill_id",
                "task_item_sha256",
                "instructions_sha256",
            ],
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

    def test_builds_v1_index_with_collection_fingerprints(self) -> None:
        index = self.index()
        result = self.validate(index)

        self.assertEqual(index["schema"], "work-execution-index/v1")
        self.assertEqual(index["task_collection_sha256"], "a" * 64)
        self.assertEqual(index["task_index_sha256"], "1" * 64)
        self.assertEqual(index["tasks"][0]["task_item_sha256"], "2" * 64)
        self.assertNotIn("task_sha256", index)
        self.assertEqual(result["schema"], "work-execution-index-validation/v1")

    def test_v1_rejects_legacy_single_file_fingerprint(self) -> None:
        index = self.index()
        index["task_sha256"] = index.pop("task_collection_sha256")
        index.pop("task_index_sha256")

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(
            context.exception.details["missing"],
            ["task_collection_sha256", "task_index_sha256"],
        )
        self.assertEqual(context.exception.details["unknown"], ["task_sha256"])

    def test_rejects_source_v1_provenance(self) -> None:
        index = self.index()
        index["source_v1_task_sha256"] = "8" * 64

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(
            context.exception.details["unknown"], ["source_v1_task_sha256"]
        )

    def test_rejects_retired_schema(self) -> None:
        index = self.index()
        index["schema"] = "work-execution-index/v2"

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_execution_index_schema")


class ExecutionInstructionAuditTests(unittest.TestCase):
    def index(self) -> dict[str, object]:
        index = build_initial_execution_index(
            {
                "requirement_id": "example",
                "spec_id": "TASK-SPEC-001",
                "tasks": [{"id": "TASK-001", "skill_id": None}],
            },
            {
                "schema": "work-task-collection-validation/v1",
                "task_collection_sha256": "a" * 64,
                "task_index_sha256": "1" * 64,
                "task_item_sha256": {"TASK-001": "2" * 64},
                "instructions_sha256": "b" * 64,
                "task_instructions_sha256": {"TASK-001": "c" * 64},
                "hierarchy_selection_sha256": "f" * 64,
                "skill_selection_sha256": "d" * 64,
                "task_skill_ids": {"TASK-001": None},
            },
        )
        audit_id = "TASK-INSTRUCTION-AUDIT-001"
        index["latest_task_instruction_audit"] = audit_id
        index["overall_status"] = "blocked"
        index["tasks"][0].update(  # type: ignore[index]
            {
                "status": "blocked",
                "status_reason": {
                    "kind": "instruction_audit",
                    "ref": audit_id,
                },
            }
        )
        return index

    def validate(self, index: dict[str, object]) -> dict[str, object]:
        return validate_execution_index(
            render_execution_index(index),
            source="test",
            expected=index,
        )

    def test_accepts_instruction_audit_fields(self) -> None:
        result = self.validate(self.index())

        self.assertEqual(result["overall_status"], "blocked")

    def test_renders_instruction_audit_field_canonically(self) -> None:
        raw = render_execution_index(dict(reversed(self.index().items())))
        payload = json.loads(
            raw.decode("utf-8")
        )

        self.assertEqual(
            list(payload)[9],
            "latest_task_instruction_audit",
        )

    def test_rejects_legacy_task_rule_audit_field(self) -> None:
        index = copy.deepcopy(self.index())
        index["latest_task_rule_audit"] = index.pop(
            "latest_task_instruction_audit"
        )

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(
            context.exception.details["unknown"],
            ["latest_task_rule_audit"],
        )

    def test_rejects_legacy_rule_audit_status_reason(self) -> None:
        index = copy.deepcopy(self.index())
        index["tasks"][0]["status_reason"]["kind"] = "rule_audit"  # type: ignore[index]

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(context.exception.code, "invalid_status_reason")

    def test_rejects_legacy_rule_audit_id(self) -> None:
        index = copy.deepcopy(self.index())
        index["latest_task_instruction_audit"] = "TASK-RULE-AUDIT-001"

        with self.assertRaises(WorkError) as context:
            self.validate(index)

        self.assertEqual(
            context.exception.code,
            "invalid_task_instruction_audit_id",
        )

if __name__ == "__main__":
    unittest.main()
