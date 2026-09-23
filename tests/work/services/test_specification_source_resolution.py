from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.models.common.errors import WorkError
from worklib.services.specification.source_resolution import (
    resolve_plan_path, resolve_repair_artifacts, resolve_reconciliation_attempt,
)


class SpecificationSourceResolutionTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.artifacts = {
            "plan": "outputs/work/custom/confirmed-plan.json",
            "task": "outputs/work/custom/tasks/index.json",
            "execution": "outputs/work/custom/execution",
        }

    def write(self, relative: str, content: dict) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(content), encoding="utf-8")

    def test_custom_plan_binding_is_found_without_caller_path(self) -> None:
        self.write(self.artifacts["plan"], {"schema": "work-plan/v1", "requirement_id": "example",
                                            "artifacts": self.artifacts})
        self.assertEqual(resolve_plan_path(self.root, "example"), self.artifacts["plan"])
        self.assertEqual(resolve_repair_artifacts(self.root, "example"), self.artifacts)

    def test_missing_duplicate_and_conflicting_plan_sources_stop(self) -> None:
        with self.assertRaises(WorkError) as missing:
            resolve_plan_path(self.root, "example")
        self.assertEqual(missing.exception.code, "spec_plan_source_missing")
        self.write(self.artifacts["plan"], {"schema": "work-plan/v1", "requirement_id": "example",
                                            "artifacts": self.artifacts})
        duplicate = dict(self.artifacts, plan="outputs/work/other-plan.json")
        self.write(duplicate["plan"], {"schema": "work-plan/v1", "requirement_id": "example",
                                       "artifacts": duplicate})
        with self.assertRaises(WorkError) as ambiguous:
            resolve_plan_path(self.root, "example")
        self.assertEqual(ambiguous.exception.code, "spec_plan_source_ambiguous")
        with self.assertRaises(WorkError) as conflicting:
            resolve_repair_artifacts(self.root, "example")
        self.assertEqual(conflicting.exception.code, "task_repair_source_ambiguous")

    def test_task_index_binding_survives_missing_plan(self) -> None:
        self.write(self.artifacts["task"], {"schema": "work-task-index/v1", "requirement_id": "example",
                                            "artifacts": self.artifacts})
        self.assertEqual(resolve_repair_artifacts(self.root, "example"), self.artifacts)

    def test_repair_rejects_conflicting_execution_binding(self) -> None:
        self.write(self.artifacts["plan"], {"schema": "work-plan/v1", "requirement_id": "example",
                                            "artifacts": self.artifacts})
        self.write("outputs/work/other-execution/index.json", {"schema": "work-execution-index/v1",
                   "requirement_id": "example"})
        with self.assertRaises(WorkError) as conflict:
            resolve_repair_artifacts(self.root, "example")
        self.assertEqual(conflict.exception.code, "task_repair_execution_binding_conflict")

    def test_reconciliation_requires_unique_latest_closed_attempt(self) -> None:
        directory = self.artifacts["execution"]
        self.write(f"{directory}/index.json", {"schema": "work-execution-index/v1",
                 "requirement_id": "example", "tasks": [{"id": "TASK-001", "latest_attempt": "ATTEMPT-002"}]})
        attempt_path = f"{directory}/TASK-001/ATTEMPT-002/attempt.json"
        self.write(attempt_path, {"attempt_id": "ATTEMPT-002", "task_id": "TASK-001", "status": "completed"})
        self.assertEqual(resolve_reconciliation_attempt(self.root, "example", 1, 2)[0], attempt_path)
        with self.assertRaises(WorkError) as old:
            resolve_reconciliation_attempt(self.root, "example", 1, 1)
        self.assertEqual(old.exception.code, "reconciliation_attempt_position")
        self.write("outputs/work/duplicate/index.json", {"schema": "work-execution-index/v1",
                   "requirement_id": "example", "tasks": []})
        with self.assertRaises(WorkError) as ambiguous:
            resolve_reconciliation_attempt(self.root, "example", 1, 2)
        self.assertEqual(ambiguous.exception.code, "reconciliation_execution_source_ambiguous")


if __name__ == "__main__":
    unittest.main()
