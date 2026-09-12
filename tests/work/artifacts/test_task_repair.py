from __future__ import annotations

import base64
import copy
import hashlib
import io
import json
import sys
from pathlib import Path
from unittest.mock import patch

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills" / "work" / "scripts"))

from cli_support import FileInputTestCase
from contracts import test_task_diagnostics as fixtures
from contracts import test_attempt as attempt_fixtures
from worklib.artifacts import task_repair
from worklib.cli import main
from worklib.contracts.execution_index import render_execution_index
from worklib.foundation.errors import WorkError
from worklib.foundation.spec_update import require_no_spec_update, state_writer


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


class TaskRepairTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.TaskDiagnosticsTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.artifacts = self.fixture.artifacts
        self.task_path = self.fixture.task_path
        self.index_path = self.fixture.index_path
        self.plan_path = self.root / self.artifacts["plan"]
        self.task_path.write_bytes(b"\xef\xbb\xbf" + self.fixture.raw)

    def request(self, stage="format"):
        return {
            "schema": "work-task-repair-request/v1", "stage": stage,
            "requirement_id": "example", "artifacts": self.artifacts,
            "expected": {
                key + "_sha256": digest(path.read_bytes())
                for key, path in (("plan", self.plan_path), ("task", self.task_path), ("index", self.index_path))
            },
            "decisions": [{"location": "/", "decision": "The user chose the reviewed candidate interpretation."}],
            "task": copy.deepcopy(self.fixture.contract),
        }

    def run_repair(self, request, operation="validate", approval=None):
        return task_repair.repair_task(
            json.dumps(request).encode("utf-8"), project_root=self.root,
            user_config_root=str(self.root), operation=operation, approved_sha256=approval,
        )

    def test_preview_is_read_only_and_apply_requires_matching_approval(self):
        request = self.request()
        before = self.fixture.snapshot()
        preview = self.run_repair(request)
        self.assertEqual(before, self.fixture.snapshot())
        self.assertEqual(base64.b64decode(preview["original_bytes_base64"]["task"]), self.task_path.read_bytes())
        self.assertIn("task", preview["diffs"])
        with self.assertRaises(WorkError):
            self.run_repair(request, "apply", "0" * 64)
        self.assertEqual(before, self.fixture.snapshot())
        result = self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(result["status"], "repaired")
        self.assertEqual(self.task_path.read_bytes(), self.fixture.raw)
        self.assertEqual(before[self.artifacts["plan"]], self.plan_path.read_bytes())
        require_no_spec_update(self.root, self.artifacts["execution"])

    def test_format_repair_preserves_missing_content_and_unknown_fields(self):
        request = self.request()
        del request["task"]["summary"]
        del request["task"]["tasks"][0]["steps"]
        del request["task"]["tasks"][0]["validations"]
        request["task"]["unknown"] = {"keep": [1, "evidence"]}
        preview = self.run_repair(request)
        self.assertFalse(preview["task_diagnostics"]["normal_use_allowed"])
        self.assertEqual(preview["task_diagnostics"]["format_status"], "passed")
        index_before = self.index_path.read_bytes()
        self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(json.loads(self.task_path.read_bytes()), request["task"])
        self.assertEqual(self.index_path.read_bytes(), index_before)

    def test_invalid_encoding_and_duplicate_json_originals_are_preserved(self):
        for raw in (b"\xff\xfeinvalid", b'{"a":1,"a":2}'):
            with self.subTest(raw=raw):
                self.task_path.write_bytes(raw)
                request = self.request()
                preview = self.run_repair(request)
                self.assertEqual(base64.b64decode(preview["original_bytes_base64"]["task"]), raw)
                self.run_repair(request, "apply", preview["approved_sha256"])
                self.assertEqual(self.task_path.read_bytes(), self.fixture.raw)

    def test_complete_repair_fixes_index_bindings(self):
        index = copy.deepcopy(self.fixture.index)
        index["task_sha256"] = "0" * 64
        self.index_path.write_bytes(render_execution_index(index))
        request = self.request("complete")
        preview = self.run_repair(request)
        self.assertIn("index", preview["diffs"])
        self.assertTrue(preview["task_diagnostics"]["normal_use_allowed"])
        self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(json.loads(self.index_path.read_bytes())["task_sha256"], digest(self.task_path.read_bytes()))

    def test_complete_repair_rejects_missing_content_and_decisions(self):
        request = self.request("complete")
        del request["task"]["summary"]
        with self.assertRaises(WorkError):
            self.run_repair(request)
        request = self.request("complete")
        request["decisions"] = []
        with self.assertRaises(WorkError):
            self.run_repair(request)

    def test_changed_review_or_artifacts_invalidate_approval(self):
        request = self.request()
        preview = self.run_repair(request)
        request["decisions"][0]["decision"] = "Changed review."
        with self.assertRaises(WorkError):
            self.run_repair(request, "apply", preview["approved_sha256"])
        request = self.request()
        preview = self.run_repair(request)
        self.task_path.write_bytes(self.task_path.read_bytes() + b"\n")
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError):
            self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(before, self.fixture.snapshot())

    def test_locks_and_active_attempts_prohibit_preview(self):
        self.fixture.index["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-001"}
        self.fixture.save_index()
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_repair(self.request())
        self.assertEqual(error.exception.code, "task_repair_diagnose_only")
        self.assertEqual(before, self.fixture.snapshot())
        del self.fixture.index["lock"]
        self.fixture.save_index()
        fixture = attempt_fixtures.AttemptContractTests()
        fixture.setUp()
        path = self.fixture.directory / "TASK-001" / "ATTEMPT-001" / "attempt.json"
        path.parent.mkdir(parents=True)
        path.write_text(json.dumps(fixture.attempt), encoding="utf-8")
        with self.assertRaises(WorkError) as error:
            self.run_repair(self.request())
        self.assertEqual(error.exception.code, "task_repair_diagnose_only")

    def test_active_writer_prohibits_preview_without_changes(self):
        with state_writer(self.root, self.artifacts["execution"]):
            before = self.fixture.snapshot()
            with self.assertRaises(WorkError) as error:
                self.run_repair(self.request())
            self.assertEqual(error.exception.code, "work_state_writer_busy")
            self.assertEqual(before, self.fixture.snapshot())

    def test_pending_other_transaction_prohibits_preview(self):
        (self.fixture.directory / ".work-task-repair-other.json").write_bytes(b"{}")
        with self.assertRaises(WorkError):
            self.run_repair(self.request())

    def test_source_change_after_lock_acquisition_is_detected(self):
        request = self.request()
        preview = self.run_repair(request)
        real = task_repair.state_writer

        from contextlib import contextmanager
        @contextmanager
        def changed(*args):
            with real(*args):
                self.task_path.write_bytes(b"external edit")
                yield

        with patch.object(task_repair, "state_writer", changed):
            with self.assertRaises(WorkError):
                self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(self.task_path.read_bytes(), b"external edit")
        self.assertFalse(list(self.fixture.directory.glob(".work-task-repair-*.json")))

    def test_interruption_blocks_use_and_recovers_exact_transaction(self):
        request = self.request("complete")
        request["task"]["summary"] = "Explicitly reviewed restored summary."
        preview = self.run_repair(request)
        real = task_repair._replace

        def interrupted(path, *args, **kwargs):
            if path == self.index_path:
                raise OSError("injected interruption after TASK replacement")
            return real(path, *args, **kwargs)

        with patch.object(task_repair, "_replace", side_effect=interrupted):
            with self.assertRaises(WorkError) as error:
                self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(error.exception.code, "task_repair_interrupted")
        with self.assertRaises(WorkError):
            require_no_spec_update(self.root, self.artifacts["execution"])
        self.assertFalse(self.fixture.diagnose()["normal_use_allowed"])
        result = self.run_repair(request, "recover", preview["approved_sha256"])
        self.assertEqual(result["status"], "recovered")
        self.assertTrue(self.fixture.diagnose()["normal_use_allowed"])
        self.assertEqual(self.run_repair(request, "recover", preview["approved_sha256"])["status"], "already_completed")

    def test_recovery_refuses_external_edits(self):
        request = self.request()
        preview = self.run_repair(request)
        with patch.object(task_repair, "_replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.run_repair(request, "apply", preview["approved_sha256"])
        self.task_path.write_bytes(b"external edit")
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError):
            self.run_repair(request, "recover", preview["approved_sha256"])
        self.assertEqual(before, self.fixture.snapshot())

    def test_partial_journal_can_only_resume_identical_approved_bytes(self):
        request = self.request()
        preview = self.run_repair(request)
        original_write = task_repair._write

        def partial(path, raw):
            if path.name.endswith(".json"):
                original_write(path, raw[:23])
                raise OSError("partial journal")
            original_write(path, raw)

        with patch.object(task_repair, "_write", side_effect=partial):
            with self.assertRaises(WorkError):
                self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(self.run_repair(request, "recover", preview["approved_sha256"])["status"], "recovered")

    def test_partial_completion_marker_recovers_without_rewriting_task(self):
        request = self.request()
        preview = self.run_repair(request)
        original_write = task_repair._write

        def partial(path, raw):
            if path.name.endswith(".done"):
                original_write(path, raw[:12])
                raise OSError("partial marker")
            original_write(path, raw)

        with patch.object(task_repair, "_write", side_effect=partial):
            with self.assertRaises(WorkError):
                self.run_repair(request, "apply", preview["approved_sha256"])
        with patch.object(task_repair, "_replace") as replace:
            self.assertEqual(self.run_repair(request, "recover", preview["approved_sha256"])["status"], "recovered")
        replace.assert_not_called()

    def test_history_is_preserved_and_changed_content_reopens_completed_rows(self):
        index = copy.deepcopy(self.fixture.index)
        index["tasks"][0].update(status="completed", latest_attempt="ATTEMPT-001")
        index["overall_status"] = "completed"
        self.index_path.write_bytes(render_execution_index(index))
        fixture = attempt_fixtures.AttemptContractTests()
        fixture.setUp()
        fixture.attempt.update(status="stopped", final_type="instructions_changed",
                               reason="Prior instructions changed.", ended_at="2026-09-01T10:05+08:00")
        path = self.fixture.directory / "TASK-001" / "ATTEMPT-001" / "attempt.json"
        path.parent.mkdir(parents=True)
        path.write_text(json.dumps(fixture.attempt), encoding="utf-8")
        history = path.read_bytes()
        request = self.request("complete")
        request["task"]["summary"] = "Restored confirmed description."
        preview = self.run_repair(request)
        self.assertEqual(preview["affected_task_ids"], ["TASK-001"])
        self.run_repair(request, "apply", preview["approved_sha256"])
        self.assertEqual(path.read_bytes(), history)
        self.assertEqual(json.loads(self.index_path.read_bytes())["tasks"][0]["status"], "pending_retry")

    def test_cli_uses_file_requests_and_json_stdout_on_success_and_failure(self):
        request = self.request()
        raw = b"\xef\xbb\xbf" + json.dumps(request).encode("utf-8")
        output = io.StringIO()
        args = [
            "--project-root", str(self.root), "task", "repair-validate",
            "--input-file", self.input_file(raw), "--user-config-root", str(self.root),
        ]
        self.assertEqual(main(args, stdout=output), 0)
        preview = json.loads(output.getvalue())
        self.assertEqual(preview["data"]["status"], "preview")
        args[3] = "repair"
        args.extend(["--approved-sha256", "0" * 64])
        output = io.StringIO()
        self.assertNotEqual(main(args, stdout=output), 0)
        self.assertEqual(json.loads(output.getvalue())["reason_code"], "task_repair_approval_changed")
