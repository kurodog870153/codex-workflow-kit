from __future__ import annotations

import sys
import unittest
from pathlib import Path

from pydantic import ValidationError


sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.models.task_collection import (
    TaskCollectionFingerprintContract,
    TaskIndexContract,
    TaskItemContract,
)
from worklib.models.task_collection import (
    TaskCollectionFingerprintContract as ModelTaskCollectionFingerprintContract,
    TaskIndexContract as ModelTaskIndexContract,
    TaskItemContract as ModelTaskItemContract,
)


def selection(*, document: bool = False) -> dict[str, object]:
    value: dict[str, object] = {
        "sources": [{"kind": "instruction", "logical_name": "task.general", "canonical_sha256": "a" * 64}],
        "references": [],
        "instructions_sha256": "b" * 64,
    }
    if not document:
        value = {"selected_paths": [], "resolved_paths": ["general"], **value}
    return value


def item() -> dict[str, object]:
    return {
        "schema": "work-task-item/v1", "id": "TASK-001", "title": "Example",
        "skill_id": None, "instruction_selection": selection(),
        "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
        "goal": "Produce the result.",
        "steps": [{"id": "STEP-001", "action": "Run.", "references": ["VAL-001"]}],
        "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "user", "criteria": "Approved."}],
    }


def index() -> dict[str, object]:
    return {
        "schema": "work-task-index/v1", "requirement_id": "example",
        "spec_id": "TASK-SPEC-001", "status": "confirmed", "title": "Example",
        "summary": "Example tasks.",
        "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
        "source_plan": {"canonical_sha256": "c" * 64, "hierarchy_selection_sha256": "d" * 64},
        "instruction_selection": selection(document=True),
        "tasks": [{"id": "TASK-001", "path": "tasks/TASK-001.json", "canonical_sha256": "e" * 64}],
        "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
    }


class TaskCollectionModelTests(unittest.TestCase):
    def test_legacy_exports_preserve_class_identity(self) -> None:
        self.assertIs(TaskItemContract, ModelTaskItemContract)
        self.assertIs(TaskIndexContract, ModelTaskIndexContract)
        self.assertIs(
            TaskCollectionFingerprintContract,
            ModelTaskCollectionFingerprintContract,
        )

    def test_v1_item_and_index_are_strict_frozen_and_canonical(self) -> None:
        item_model = TaskItemContract.model_validate(item())
        index_model = TaskIndexContract.model_validate(index())
        rendered_item = item_model.to_canonical_dict()
        self.assertEqual(list(rendered_item), list(TaskItemContract.canonical_order[:7]) + ["steps", "validations"])
        self.assertIsNone(rendered_item["skill_id"])
        self.assertNotIn("dependencies", rendered_item)
        self.assertEqual(list(index_model.to_canonical_dict()), [field for field in TaskIndexContract.canonical_order if field in index()])
        with self.assertRaises(ValidationError):
            item_model.title = "changed"  # type: ignore[misc]

    def test_rejects_retired_schema_extra_fields_and_coercion(self) -> None:
        for value in (
            {**item(), "schema": "work-task-item/v2"},
            {**item(), "unknown": True},
            {**item(), "title": 1},
        ):
            with self.subTest(value=value), self.assertRaises(ValidationError):
                TaskItemContract.model_validate(value)

        with self.assertRaises(ValidationError):
            TaskIndexContract.model_validate({**index(), "schema": "work-task-index/v2"})

    def test_nested_models_forbid_unknown_fields(self) -> None:
        value = item()
        value["traceability"] = {**value["traceability"], "unknown": True}  # type: ignore[dict-item]
        with self.assertRaises(ValidationError):
            TaskItemContract.model_validate(value)

    def test_collection_fingerprint_uses_v1(self) -> None:
        value = {
            "schema": "work-task-collection-fingerprint/v1",
            "task_index_sha256": "a" * 64,
            "items": [{"id": "TASK-001", "task_item_sha256": "b" * 64}],
        }
        model = TaskCollectionFingerprintContract.model_validate(value)
        self.assertEqual(model.to_canonical_dict(), value)
        with self.assertRaises(ValidationError):
            TaskCollectionFingerprintContract.model_validate({**value, "schema": "work-task-collection-fingerprint/v2"})


if __name__ == "__main__":
    unittest.main()
