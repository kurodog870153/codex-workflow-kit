from __future__ import annotations

import shutil
import sys
import unittest
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(PROJECT_ROOT / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_task_collection as fixtures
from worklib.business_services.instruction import apply_instruction_migration, preview_instruction_migration
from worklib.business_services.task import load_task_collection
from worklib.services.attempt import build_initial_execution_index, render_execution_index
from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.json_contract import parse_json_contract


class InstructionMigrationFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture = fixtures.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        fixture.setUp(); self.addCleanup(fixture.doCleanups)
        self.fixture = fixture; self.root = fixture.root
        self.skill_root = self.root / "work-skill"
        shutil.copytree(PROJECT_ROOT / "skills" / "work", self.skill_root)
        validation = load_task_collection(self.root, str(self.root), fixture.index_path)
        self.artifacts = validation["collection_contract"]["artifacts"]
        self.execution = self.root / self.artifacts["execution"] / "index.json"
        self.execution.parent.mkdir(parents=True, exist_ok=True)
        self.execution.write_bytes(render_execution_index(build_initial_execution_index(validation["collection_contract"], validation)))

    def test_preview_apply_and_idempotent_reapply_use_new_router(self) -> None:
        history = self.execution.parent / "TASK-001" / "ATTEMPT-001" / "attempt.json"
        history.parent.mkdir(parents=True); history.write_bytes(b"history\n")
        before_history = history.read_bytes()
        preview = preview_instruction_migration(self.root, self.skill_root, "example")
        self.assertEqual(preview["status"], "migration_required")
        result = apply_instruction_migration(self.root, self.skill_root, "example", preview["approved_sha256"])
        self.assertEqual(result["status"], "updated")
        self.assertEqual(history.read_bytes(), before_history)
        self.assertEqual(preview_instruction_migration(self.root, self.skill_root, "example")["status"], "current")
        repeated = apply_instruction_migration(self.root, self.skill_root, "example", preview["approved_sha256"])
        self.assertEqual(repeated["status"], "already_completed")
        execution = parse_json_contract(self.execution.read_bytes(), source="execution")
        self.assertEqual(execution["instruction_selection_manifest"]["router_compatibility_revision"], 3)

    def test_apply_rejects_router_source_drift(self) -> None:
        preview = preview_instruction_migration(self.root, self.skill_root, "example")
        source = self.skill_root / "references" / "instruction-loading.md"
        source.write_bytes(source.read_bytes() + b"\nDrift.\n")
        with self.assertRaises(WorkError) as caught:
            apply_instruction_migration(
                self.root, self.skill_root, "example", preview["approved_sha256"],
            )
        self.assertEqual(caught.exception.code, "instruction_migration_approval_changed")

    def test_active_attempt_blocks_without_changing_artifacts_or_history(self) -> None:
        execution = parse_json_contract(self.execution.read_bytes(), source="execution")
        execution["tasks"][0]["status"] = "in_progress"
        execution["tasks"][0]["latest_attempt"] = "ATTEMPT-001"
        execution["overall_status"] = "in_progress"
        self.execution.write_bytes(render_execution_index(execution))
        history = self.execution.parent / "TASK-001" / "ATTEMPT-001" / "attempt.json"
        history.parent.mkdir(parents=True); history.write_bytes(b"active snapshot\n")
        paths = [self.root / self.artifacts["plan"], self.root / self.artifacts["task"], self.execution, history]
        before = {path: path.read_bytes() for path in paths}
        preview = preview_instruction_migration(self.root, self.skill_root, "example")
        self.assertEqual(preview["status"], "review_required")
        self.assertEqual(preview["files"], [])
        self.assertEqual({path: path.read_bytes() for path in paths}, before)


if __name__ == "__main__":
    unittest.main()
