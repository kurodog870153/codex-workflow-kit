from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.foundation.errors import WorkError
from worklib.foundation.fingerprint import raw_sha256
from worklib.services.specification import execution_history_fingerprints, rebuild_execution_index


class SpecificationServiceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def test_history_fingerprints_preserve_relative_paths_and_bytes(self):
        first = self.root / "execution/TASK-001/attempts/ATTEMPT-001.json"
        second = self.root / "execution/TASK-002/corrections/CORRECTION-001.json"
        first.parent.mkdir(parents=True)
        second.parent.mkdir(parents=True)
        first.write_bytes(b"first\n")
        second.write_bytes(b"second\n")
        self.assertEqual(execution_history_fingerprints(self.root, "execution"), {
            "execution/TASK-001/attempts/ATTEMPT-001.json": raw_sha256(b"first\n"),
            "execution/TASK-002/corrections/CORRECTION-001.json": raw_sha256(b"second\n"),
        })

    def test_history_rejects_non_directory_task_entry(self):
        directory = self.root / "execution"
        directory.mkdir()
        (directory / "TASK-001").write_text("invalid")
        with self.assertRaises(WorkError) as caught:
            execution_history_fingerprints(self.root, "execution")
        self.assertEqual(caught.exception.code, "spec_update_history_layout")

    def test_rebuild_preserves_history_and_reopens_affected_rows(self):
        generated = {"tasks": [
            {"id": "TASK-001", "status": "pending", "skill_id": "new", "instructions_sha256": "1" * 64},
            {"id": "TASK-002", "status": "pending", "skill_id": None, "instructions_sha256": "2" * 64},
        ], "overall_status": "pending"}
        old = {"tasks": [
            {"id": "TASK-001", "status": "completed", "skill_id": "old", "instructions_sha256": "0" * 64,
             "latest_attempt": "ATTEMPT-001"},
            {"id": "TASK-002", "status": "blocked", "skill_id": "old", "instructions_sha256": "0" * 64},
        ]}
        with patch("worklib.services.specification.build_initial_execution_index", return_value=generated):
            result = rebuild_execution_index(old, {}, {}, ["TASK-001", "TASK-002"], "TASK-CHANGE-002")
        self.assertEqual([row["status"] for row in result["tasks"]], ["pending_retry", "blocked"])
        self.assertEqual(result["tasks"][0]["latest_attempt"], "ATTEMPT-001")
        self.assertEqual(result["tasks"][0]["skill_id"], "new")
        self.assertEqual(result["tasks"][1]["status_reason"], {"kind": "task_change", "ref": "TASK-CHANGE-002"})

    def test_rebuild_rejects_affected_cancelled_task(self):
        generated = {"tasks": [{"id": "TASK-001", "status": "pending", "skill_id": None,
                                  "instructions_sha256": "1" * 64}], "overall_status": "pending"}
        old = {"tasks": [{"id": "TASK-001", "status": "cancelled", "skill_id": None,
                           "instructions_sha256": "0" * 64}]}
        with patch("worklib.services.specification.build_initial_execution_index", return_value=generated):
            with self.assertRaises(WorkError) as caught:
                rebuild_execution_index(old, {}, {}, ["TASK-001"], "TASK-CHANGE-002")
        self.assertEqual(caught.exception.code, "spec_update_cancelled_task")
