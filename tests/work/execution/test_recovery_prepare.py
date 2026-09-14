from __future__ import annotations

import copy
import io
import json
import sys
from pathlib import Path
from unittest.mock import patch

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills/work/scripts"))

from cli_support import FileInputTestCase
from contracts import test_task_diagnostics as fixtures
from worklib.cli import main
from worklib.contracts.attempt import render_attempt_contract
from worklib.contracts.execution_index import render_execution_index
from worklib.execution import recovery_prepare
from worklib.execution.recovery import recover_execution
from worklib.foundation.errors import WorkError
from worklib.foundation.spec_update import state_writer


class RecoveryPreparationTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.TaskDiagnosticsTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.index = copy.deepcopy(self.fixture.index)
        self.attempt = {"schema": "work-attempt/v1", "attempt_id": "ATTEMPT-001", "task_id": "TASK-001",
            "task_spec_id": self.index["task_spec_id"], "skill_id": None, "status": "in_progress",
            "task_sha256": self.index["task_sha256"],
            "task_instructions_sha256": self.index["tasks"][0]["instructions_sha256"],
            "execute_instructions_sha256": "c" * 64,
            "hierarchy_selection_sha256": self.index["hierarchy_selection_sha256"],
            "execute_skill_selection_sha256": self.index["skill_selection_sha256"],
            "started_at": "2026-09-01T10:00+08:00", "records": []}
        self.index["tasks"][0].update(status="in_progress", latest_attempt="ATTEMPT-001")
        self.index["overall_status"] = "in_progress"
        self.index["lock"] = {"kind": "execution", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
                              "execute_instructions_sha256": "c" * 64}
        self.attempt_path = self.fixture.directory / "TASK-001/ATTEMPT-001/attempt.json"
        self.attempt_path.parent.mkdir(parents=True)
        self.save()
        self.common = dict(project_root=self.root, user_config_root=str(self.root),
            raw_task_path=self.fixture.artifacts["task"], raw_execution_dir=self.fixture.artifacts["execution"],
            task_id="TASK-001", source="test")

    def save(self):
        self.fixture.index_path.write_bytes(render_execution_index(self.index))
        self.attempt_path.write_bytes(render_attempt_contract(self.attempt, project_root=self.root))

    def request(self, transaction="record_begin"):
        return {"schema": "work-execution-recovery-prepare-request/v1", "transaction": transaction,
                "attempt_id": "ATTEMPT-001"}

    def prepare(self, transaction="record_begin", **overrides):
        return recovery_prepare.prepare_execution_recovery(json.dumps(self.request(transaction)).encode(),
                                                           **{**self.common, **overrides})

    def begin_file(self):
        prepared = copy.deepcopy(self.index)
        prepared["lock"]["record_id"] = "VAL-001"
        path = self.fixture.directory / ".work-record-begin-TASK-001-ATTEMPT-001-VAL-001.tmp"
        path.write_bytes(render_execution_index(prepared))
        return path

    def test_read_only_request_can_drive_existing_recovery(self):
        path = self.begin_file()
        before = self.fixture.snapshot()
        result = self.prepare()
        self.assertEqual(result["request"]["transaction_files"], [path.name])
        self.assertFalse(result["recovery_authorized"])
        self.assertEqual(result["recovery_validation"], "requires_authorized_recover")
        self.assertEqual(len(result["evidence"]), 5)
        self.assertEqual(before, self.fixture.snapshot())
        recovered = recover_execution(json.dumps(result["request"]).encode(), **self.common)
        self.assertEqual(recovered["lock_status"], "record_reserved")

    def test_cli_does_not_create_writer_mutex_or_change_evidence(self):
        self.begin_file()
        before = self.fixture.snapshot()
        out = io.StringIO()
        args = self.input_arguments(["--project-root", str(self.root), "execute", "recovery-prepare",
            "--task-path", self.fixture.artifacts["task"], "--execution-dir", self.fixture.artifacts["execution"],
            "--task-id", "TASK-001", "--user-config-root", str(self.root), "--input-file", "request.json"],
            json.dumps(self.request()))
        self.assertEqual(main(args, stdout=out), 0, out.getvalue())
        self.assertEqual(json.loads(out.getvalue())["data"]["schema"], "work-execution-recovery-prepare/v1")
        self.assertEqual(before, self.fixture.snapshot())

    def test_foreign_and_attempt_start_files_are_not_silently_filtered(self):
        self.begin_file()
        other = self.fixture.directory / ".work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp"
        other.write_bytes(b"preserved")
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError) as error:
            self.prepare()
        self.assertEqual(error.exception.code, "recovery_prepare_mixed_transactions")
        self.assertEqual(before, self.fixture.snapshot())

    def test_empty_inventory_requires_supported_post_write_state(self):
        with self.assertRaises(WorkError):
            self.prepare()
        with self.assertRaises(WorkError):
            self.prepare("attempt_close")
        self.attempt.update(status="stopped", final_type="instructions_changed", reason="Reviewed stop.",
                            ended_at="2026-09-01T10:05+08:00")
        self.save()
        result = self.prepare("attempt_close")
        self.assertEqual(result["request"]["transaction_files"], [])

    def test_record_finish_after_attempt_write_accepts_empty_inventory(self):
        self.index["lock"]["record_id"] = "VAL-001"
        self.attempt["records"] = [{"id": "VAL-001", "kind": "validation", "outcome": "passed", "evidence": "Reviewed."}]
        self.save()
        self.assertEqual(self.prepare("record_finish")["request"]["transaction_files"], [])

    def test_wrong_attempt_lock_and_unknown_task_are_rejected(self):
        self.begin_file()
        with self.assertRaises(WorkError):
            self.prepare(task_id="TASK-999")
        self.index["lock"]["execute_instructions_sha256"] = "d" * 64
        self.save()
        with self.assertRaises(WorkError) as error:
            self.prepare()
        self.assertEqual(error.exception.code, "recovery_prepare_lock")

    def test_changed_inventory_or_bytes_is_rejected(self):
        path = self.begin_file()
        original = recovery_prepare._inventory
        calls = 0
        def changed(*args):
            nonlocal calls
            calls += 1
            if calls == 2:
                path.write_bytes(path.read_bytes() + b"\n")
            return original(*args)
        with patch.object(recovery_prepare, "_inventory", side_effect=changed):
            with self.assertRaises(WorkError) as error:
                self.prepare()
        self.assertEqual(error.exception.code, "recovery_prepare_source_changed")

    def test_partial_file_and_active_writer_are_rejected(self):
        path = self.begin_file()
        with state_writer(self.root, self.fixture.artifacts["execution"]):
            with self.assertRaises(WorkError):
                self.prepare()
        path.write_bytes(b'{"partial":')
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError):
            self.prepare()
        self.assertEqual(before, self.fixture.snapshot())
