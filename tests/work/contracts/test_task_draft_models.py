from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from pydantic import ValidationError


sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.contracts.task_draft_models import TaskDraftContract, TaskPlanningIndexContract


SOURCE = {
    "plan_sha256": "a" * 64,
    "hierarchy_selection_sha256": "b" * 64,
    "skill_selection_sha256": "c" * 64,
}


def planning_index() -> dict[str, object]:
    return {
        "schema": "work-task-planning-index/v1", "requirement_id": "example",
        "revision": 1, "source": SOURCE, "current_task_id": None,
        "tasks": [{
            "id": "TASK-001", "title": "Example", "goal": "Deliver.",
            "scope": ["Implementation"], "skill_id": None, "dependencies": [],
            "status": "planned", "boundary_revision": 1, "instructions_sha256": "d" * 64,
        }],
    }


def draft() -> dict[str, object]:
    return {
        "schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001",
        "revision": 1, "boundary_revision": 1, "source": SOURCE,
        "instructions_sha256": "d" * 64, "status": "refined", "notes": [],
        "confirmed_decisions": [], "tentative": [], "open_questions": [],
        "next_discussion_point": None,
    }


class TaskDraftModelTests(unittest.TestCase):
    def test_required_nullable_fields_remain_canonical(self) -> None:
        index_value = TaskPlanningIndexContract.model_validate(planning_index()).to_canonical_dict()
        draft_value = TaskDraftContract.model_validate(draft()).to_canonical_dict()

        self.assertIsNone(index_value["current_task_id"])
        self.assertIsNone(index_value["tasks"][0]["skill_id"])
        self.assertIsNone(draft_value["next_discussion_point"])
        self.assertNotIn("retired_task_ids", index_value)
        self.assertNotIn("task_candidate", draft_value)

    def test_models_are_strict_nested_and_frozen(self) -> None:
        value = planning_index()
        value["revision"] = "1"
        with self.assertRaises(ValidationError):
            TaskPlanningIndexContract.model_validate(value)

        nested = planning_index()
        nested["tasks"][0]["unknown"] = True  # type: ignore[index]
        with self.assertRaises(ValidationError):
            TaskPlanningIndexContract.model_validate(nested)

        model = TaskDraftContract.model_validate(draft())
        with self.assertRaises(ValidationError):
            model.status = "in_progress"  # type: ignore[misc]

    def test_wrong_schema_and_extra_fields_are_rejected(self) -> None:
        for value in (
            {**draft(), "schema": "work-task-draft/v2"},
            {**draft(), "unknown": True},
        ):
            with self.subTest(value=value), self.assertRaises(ValidationError):
                TaskDraftContract.model_validate(value)


if __name__ == "__main__":
    unittest.main()
