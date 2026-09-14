from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.task import validate_task_json_contract
from worklib.contracts.task_draft import (
    DRAFT_SCHEMA,
    INDEX_SCHEMA,
    validate_task_draft,
    validate_task_planning_index,
)
from worklib.foundation.errors import WorkError


class TaskDraftContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.index = {
            "schema": INDEX_SCHEMA,
            "requirement_id": "example",
            "revision": 1,
            "source": {
                "plan_sha256": "a" * 64,
                "hierarchy_selection_sha256": "b" * 64,
                "skill_selection_sha256": "c" * 64,
            },
            "current_task_id": "TASK-001",
            "tasks": [{
                "id": "TASK-001", "title": "Update source", "goal": "Produce the result.",
                "scope": ["Source and its validation."], "skill_id": None,
                "dependencies": [], "status": "in_progress", "boundary_revision": 1,
                "instructions_sha256": "d" * 64,
            }],
        }
        self.draft = {
            "schema": DRAFT_SCHEMA, "requirement_id": "example", "task_id": "TASK-001",
            "revision": 1, "boundary_revision": 1, "source": copy.deepcopy(self.index["source"]),
            "instructions_sha256": "d" * 64, "status": "in_progress", "notes": [],
            "confirmed_decisions": [{"statement": "Keep the API.", "rationale": "Preserve callers."}],
            "tentative": ["Consider a focused regression test."],
            "open_questions": ["Which input should the test cover?"],
            "next_discussion_point": "Confirm the regression input.",
        }

    def assert_rejected(self, operation, code: str) -> None:
        with self.assertRaises(WorkError) as context:
            operation()
        self.assertEqual(context.exception.code, code)

    def validate(self):
        return validate_task_draft(self.draft, index=self.index)

    def test_optional_index_selection_preserves_legacy_drafts(self):
        self.assertEqual(self.validate()["status"], "valid")
        self.index["tasks"][0]["instruction_selection"] = {"selected_paths": [], "references": []}
        self.assertEqual(self.validate()["status"], "valid")
        self.index["tasks"][0]["instruction_selection"]["references"] = ["same", "same"]
        self.assert_rejected(self.validate, "invalid_source_selection")

    def test_incomplete_discussion_is_valid_without_formal_readiness(self) -> None:
        before = copy.deepcopy((self.index, self.draft))
        result = self.validate()
        self.assertEqual(result["status"], "valid")
        self.assertEqual(result["planning_status"], "in_progress")
        self.assertNotIn("readiness", result)
        self.assertEqual((self.index, self.draft), before)

    def test_index_does_not_require_loading_drafts(self) -> None:
        self.index["tasks"][0]["status"] = "planned"
        self.index["current_task_id"] = None
        self.assertEqual(validate_task_planning_index(self.index)["task_count"], 1)

    def test_unrelated_index_revision_does_not_invalidate_draft(self) -> None:
        self.index["revision"] = 8
        self.assertEqual(self.validate()["status"], "valid")

    def test_refined_is_discussion_progress_only(self) -> None:
        self.index["tasks"][0]["status"] = self.draft["status"] = "refined"
        self.draft.update(tentative=[], open_questions=[], next_discussion_point=None)
        self.assertEqual(self.validate()["planning_status"], "refined")

    def test_refined_rejects_unresolved_content(self) -> None:
        self.index["tasks"][0]["status"] = self.draft["status"] = "refined"
        for field, pending in (("tentative", ["Maybe"]), ("open_questions", ["Which?"]), ("next_discussion_point", "Continue")):
            with self.subTest(field=field):
                self.draft.update(tentative=[], open_questions=[], next_discussion_point=None)
                self.draft[field] = pending
                self.assert_rejected(self.validate, "unfinished_refined_draft")

    def test_unfinished_requires_resume_point(self) -> None:
        for status in ("in_progress", "needs_review"):
            with self.subTest(status=status):
                self.index["tasks"][0]["status"] = self.draft["status"] = status
                self.draft["next_discussion_point"] = None
                self.assert_rejected(self.validate, "missing_draft_resume_point")

    def test_source_and_requirement_mismatches_are_rejected(self) -> None:
        for field in self.draft["source"]:
            with self.subTest(field=field):
                original = self.draft["source"][field]
                self.draft["source"][field] = "e" * 64
                self.assert_rejected(self.validate, "draft_source_mismatch")
                self.draft["source"][field] = original
        self.draft["requirement_id"] = "other"
        self.assert_rejected(self.validate, "draft_source_mismatch")

    def test_changed_task_boundary_instructions_and_status_are_rejected(self) -> None:
        for field, value in (("boundary_revision", 2), ("instructions_sha256", "e" * 64), ("status", "needs_review")):
            with self.subTest(field=field):
                original = self.index["tasks"][0][field]
                self.index["tasks"][0][field] = value
                self.assert_rejected(self.validate, "draft_index_mismatch")
                self.index["tasks"][0][field] = original

    def test_unknown_task_and_resume_target_are_rejected(self) -> None:
        self.draft["task_id"] = "TASK-002"
        self.assert_rejected(self.validate, "draft_task_not_in_index")
        self.index["current_task_id"] = "TASK-002"
        self.assert_rejected(lambda: validate_task_planning_index(self.index), "unknown_current_task")

    def test_duplicate_task_is_rejected(self) -> None:
        self.index["tasks"].append(copy.deepcopy(self.index["tasks"][0]))
        self.assert_rejected(lambda: validate_task_planning_index(self.index), "duplicate_draft_task_id")

    def test_dependency_validation_and_order(self) -> None:
        second = copy.deepcopy(self.index["tasks"][0])
        second.update(id="TASK-002", dependencies=["TASK-001"], status="planned")
        self.index["tasks"].insert(0, second)
        self.assertEqual(validate_task_planning_index(self.index)["task_order"], ["TASK-001", "TASK-002"])
        for dependencies, code in ((["TASK-001", "TASK-001"], "duplicate_draft_dependency"), (["TASK-002"], "invalid_task_dependency"), (["TASK-003"], "invalid_task_dependency")):
            with self.subTest(dependencies=dependencies):
                second["dependencies"] = dependencies
                self.assert_rejected(lambda: validate_task_planning_index(self.index), code)
        second["dependencies"] = ["TASK-001"]
        self.index["tasks"][1]["dependencies"] = ["TASK-002"]
        self.assert_rejected(lambda: validate_task_planning_index(self.index), "cyclic_task_dependency")

    def test_bad_revisions_ids_fingerprints_and_statuses_are_rejected(self) -> None:
        for field, value, code in (
            ("revision", True, "invalid_draft_revision"),
            ("revision", 0, "invalid_draft_revision"),
            ("boundary_revision", "1", "invalid_draft_revision"),
            ("task_id", "TASK-1", "invalid_draft_task_id"),
            ("instructions_sha256", "D" * 64, "invalid_sha256"),
            ("status", "confirmed", "invalid_draft_status"),
            ("status", {}, "invalid_draft_status"),
            ("notes", "text", "invalid_draft_array"),
            ("schema", "work-task/v1", "invalid_draft_schema"),
        ):
            with self.subTest(field=field, value=value):
                original = self.draft[field]
                self.draft[field] = value
                self.assert_rejected(self.validate, code)
                self.draft[field] = original

    def test_missing_fields_and_authorization_fields_are_rejected(self) -> None:
        del self.draft["open_questions"]
        self.assert_rejected(self.validate, "invalid_object_fields")
        self.draft["open_questions"] = []
        self.draft["approved"] = True
        self.assert_rejected(self.validate, "invalid_object_fields")

    def test_confirmed_decision_requires_rationale(self) -> None:
        self.draft["confirmed_decisions"][0]["rationale"] = " "
        self.assert_rejected(self.validate, "empty_text_value")

    def test_draft_cannot_be_used_as_formal_task(self) -> None:
        import json

        self.assert_rejected(
            lambda: validate_task_json_contract(
                json.dumps(self.draft).encode("utf-8"), source="test",
                actual_task_path="outputs/work/tasks/example/task.json",
                project_root=Path.cwd(), user_config_root=str(Path.cwd()),
            ),
            "invalid_object_fields",
        )

    def test_version_reference_requires_valid_hash_and_existing_save(self) -> None:
        reference = {"save_revision": 1, "revision": 1, "sha256": "a" * 64}
        self.index["tasks"][0]["draft_ref"] = reference
        self.validate()
        reference["revision"] = 2
        self.assert_rejected(self.validate, "draft_index_mismatch")
        reference["revision"] = 1
        reference["save_revision"] = 2
        self.assert_rejected(self.validate, "invalid_draft_reference")
        reference["save_revision"] = 1
        reference["sha256"] = "bad"
        self.assert_rejected(self.validate, "invalid_sha256")


if __name__ == "__main__":
    unittest.main()
