from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts import test_task_collection
from worklib.business_services.task import creation as task_artifact
from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.json_contract import render_json_contract


class TaskCollectionCreateTests(unittest.TestCase):
    def setUp(self):
        fixture = test_task_collection.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.root = fixture.root
        self.index_path = fixture.index_path
        self.execution = fixture.index["artifacts"]["execution"]
        self.contract = {key: value for key, value in fixture.index.items() if key not in {"schema", "tasks"}}
        self.contract["schema"] = "work-task-collection-projection/v1"
        self.contract["tasks"] = [{key: value for key, value in fixture.item.items() if key != "schema"}]
        index_file = self.root / self.index_path
        index_file.parent.joinpath("tasks/TASK-001.json").unlink()
        index_file.parent.joinpath("tasks").rmdir()
        index_file.unlink()
        index_file.parent.rmdir()
        self.raw = render_json_contract(self.contract)
        self.common = {
            "source": "test", "raw_plan_path": self.contract["artifacts"]["plan"],
            "raw_task_path": self.index_path, "raw_execution_dir": self.execution,
            "project_root": self.root, "user_config_root": str(self.root),
        }

    def test_create_writes_complete_collection_and_blocks_duplicate(self):
        drafts = (self.root / self.index_path).parent / "drafts"
        drafts.mkdir(parents=True)
        drafts.joinpath("index.json").write_bytes(b"draft planning\n")
        result = task_artifact.create_task_artifacts(self.raw, **self.common)
        self.assertEqual(result["schema"], "work-task-create/v1")
        self.assertTrue((self.root / self.index_path).is_file())
        self.assertTrue((self.root / self.index_path).parent.joinpath("tasks/TASK-001.json").is_file())
        self.assertTrue((self.root / self.execution).joinpath("index.json").is_file())
        self.assertEqual(drafts.joinpath("index.json").read_bytes(), b"draft planning\n")
        with self.assertRaises(WorkError) as caught:
            task_artifact.create_task_artifacts(self.raw, **self.common)
        self.assertEqual(caught.exception.code, "task_create_target_exists")

    def test_create_rejects_unknown_collection_entry(self):
        collection = (self.root / self.index_path).parent
        collection.mkdir()
        collection.joinpath("unexpected.json").write_bytes(b"{}\n")
        with self.assertRaises(WorkError) as caught:
            task_artifact.create_task_artifacts(self.raw, **self.common)
        self.assertEqual(caught.exception.code, "task_create_target_exists")
        self.assertEqual(caught.exception.details["unexpected_collection_entries"], ["unexpected.json"])

    def test_recover_completes_interrupted_create_and_rejects_conflict(self):
        original = task_artifact._write_exclusive
        def interrupt(path, content, **kwargs):
            if path == self.root / self.execution / "index.json":
                raise WorkError(task_artifact.ExitCode.IO_FAILURE, "simulated", "simulated")
            return original(path, content, **kwargs)
        with patch.object(task_artifact, "_write_exclusive", side_effect=interrupt), self.assertRaises(WorkError):
            task_artifact.create_task_artifacts(self.raw, **self.common)
        result = task_artifact.recover_task_create(self.raw, **self.common)
        self.assertEqual(result["status"], "recovered")
        self.assertEqual(task_artifact.recover_task_create(self.raw, **self.common)["status"], "already_completed")
        (self.root / self.index_path).parent.joinpath("tasks/TASK-001.json").write_bytes(b"conflict")
        with self.assertRaises(WorkError) as caught:
            task_artifact.recover_task_create(self.raw, **self.common)
        self.assertEqual(caught.exception.code, "unrecoverable_task_create_state")

    def test_recover_completes_interruption_before_first_item(self):
        original = task_artifact._write_exclusive
        def interrupt(path, content, **kwargs):
            if path.name == "TASK-001.json":
                raise WorkError(task_artifact.ExitCode.IO_FAILURE, "simulated", "simulated")
            return original(path, content, **kwargs)
        with patch.object(task_artifact, "_write_exclusive", side_effect=interrupt), self.assertRaises(WorkError):
            task_artifact.create_task_artifacts(self.raw, **self.common)
        result = task_artifact.recover_task_create(self.raw, **self.common)
        self.assertEqual(result["status"], "recovered")

    def test_recover_completes_interruption_before_formal_index(self):
        original = task_artifact._write_exclusive
        def interrupt(path, content, **kwargs):
            if path == self.root / self.index_path:
                raise WorkError(task_artifact.ExitCode.IO_FAILURE, "simulated", "simulated")
            return original(path, content, **kwargs)
        with patch.object(task_artifact, "_write_exclusive", side_effect=interrupt), self.assertRaises(WorkError):
            task_artifact.create_task_artifacts(self.raw, **self.common)
        result = task_artifact.recover_task_create(self.raw, **self.common)
        self.assertEqual(result["status"], "recovered")

    def test_recover_rejects_unknown_collection_entry_before_writing(self):
        collection = (self.root / self.index_path).parent
        collection.mkdir()
        collection.joinpath("unexpected.json").write_bytes(b"{}\n")
        with self.assertRaises(WorkError) as caught:
            task_artifact.recover_task_create(self.raw, **self.common)
        self.assertEqual(caught.exception.code, "unrecoverable_task_create_state")
        self.assertFalse(collection.joinpath("tasks").exists())


if __name__ == "__main__":
    unittest.main()
