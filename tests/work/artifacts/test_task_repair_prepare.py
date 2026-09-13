from __future__ import annotations

import base64
import copy
import io
import json
import sys
from pathlib import Path
from unittest.mock import patch

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills" / "work" / "scripts"))

from cli_support import FileInputTestCase
from artifacts import test_task_repair as fixtures
from worklib.artifacts import task_repair_prepare
from worklib.cli import main
from worklib.contracts.execution_index import render_execution_index
from worklib.foundation.errors import WorkError
from worklib.foundation.spec_update import state_writer


class TaskRepairPreparationTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.TaskRepairTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root

    def request(self, stage="format"):
        full = self.fixture.request(stage)
        return {**{key: full[key] for key in ("stage", "requirement_id", "artifacts", "decisions")},
                "schema": "work-task-repair-prepare-request/v1"}

    def prepare(self, request, **kwargs):
        return task_repair_prepare.prepare_task_repair(json.dumps(request).encode(),
            project_root=self.root, user_config_root=str(self.root), **kwargs)

    def test_format_preparation_revalidates_with_identical_approval_without_writes(self):
        before = self.fixture.fixture.snapshot()
        result = self.prepare(self.request())
        self.assertEqual(result["schema"], "work-task-repair-prepare/v1")
        self.assertEqual(result["request"]["task"], self.fixture.fixture.contract)
        self.assertEqual(result["request"]["expected"], self.fixture.request()["expected"])
        self.assertEqual(self.fixture.run_repair(result["request"]), result["preview"])
        self.assertEqual(before, self.fixture.fixture.snapshot())

    def test_format_candidate_preserves_unknowns_nulls_and_missing_fields(self):
        task = copy.deepcopy(self.fixture.fixture.contract)
        del task["summary"]
        del task["tasks"][0]["steps"]
        task["unknown"] = {"evidence": [None, "keep"]}
        self.fixture.task_path.write_bytes(b"\xef\xbb\xbf" + json.dumps(task).encode())
        result = self.prepare(self.request())
        self.assertEqual(result["request"]["task"], task)
        self.assertFalse(result["preview"]["task_diagnostics"]["normal_use_allowed"])
        self.assertNotIn("index", result["preview"]["diffs"])

    def test_complete_edits_restore_missing_fields_and_derive_index(self):
        task = copy.deepcopy(self.fixture.fixture.contract)
        summary, steps = task.pop("summary"), task["tasks"][0].pop("steps")
        task["legacy_field"] = {"retained": "reviewed evidence"}
        self.fixture.task_path.write_text(json.dumps(task))
        request = self.request("complete")
        request["edits"] = [
            {"operation": "add", "field": "summary", "after": summary},
            {"operation": "add", "task_id": "TASK-001", "field": "steps", "after": steps},
            {"operation": "remove", "field": "legacy_field", "before": task["legacy_field"]},
            {"operation": "replace", "field": "title", "before": task["title"], "after": "Reviewed title"},
        ]
        before = self.fixture.fixture.snapshot()
        result = self.prepare(request)
        self.assertTrue(result["preview"]["task_diagnostics"]["normal_use_allowed"])
        self.assertEqual(result["request"]["task"]["summary"], summary)
        self.assertEqual(result["request"]["task"]["tasks"][0]["steps"], steps)
        self.assertNotIn("legacy_field", result["request"]["task"])
        self.assertEqual(self.fixture.run_repair(result["request"]), result["preview"])
        self.assertEqual(before, self.fixture.fixture.snapshot())

    def test_complete_stage_can_prepare_index_only_repair(self):
        self.fixture.task_path.write_bytes(self.fixture.fixture.raw)
        index = copy.deepcopy(self.fixture.fixture.index)
        index["task_sha256"] = "0" * 64
        self.fixture.index_path.write_bytes(render_execution_index(index))
        result = self.prepare(self.request("complete"))
        self.assertEqual(set(result["preview"]["diffs"]), {"index"})
        self.assertEqual(self.fixture.run_repair(result["request"]), result["preview"])

    def test_ambiguous_original_requires_explicit_candidate_and_preserves_bytes(self):
        for raw, code in ((b"\xffinvalid", "invalid_utf8"), (b'{"a":1,"a":2}', "duplicate_json_key"),
                          (b'{"missing":', "invalid_json_contract")):
            with self.subTest(raw=raw):
                self.fixture.task_path.write_bytes(raw)
                request = self.request()
                before = self.fixture.fixture.snapshot()
                with self.assertRaises(WorkError) as error:
                    self.prepare(request)
                self.assertEqual(error.exception.code, code)
                self.assertIn("task_diagnostics", error.exception.details)
                request["task"] = copy.deepcopy(self.fixture.fixture.contract)
                result = self.prepare(request)
                self.assertEqual(base64.b64decode(result["preview"]["original_bytes_base64"]["task"]), raw)
                self.assertEqual(before, self.fixture.fixture.snapshot())

    def test_stale_duplicate_ambiguous_and_overlapping_edits_are_rejected(self):
        task = self.fixture.fixture.contract
        replace = {"operation": "replace", "field": "title", "before": task["title"], "after": "Reviewed"}
        row = {"operation": "add", "task_id": "TASK-001", "field": "unknown", "after": None}
        whole = {"operation": "replace", "field": "tasks", "before": task["tasks"], "after": []}
        for edits in ([dict(replace, before="stale")], [replace, replace], [whole, row], [row, whole],
                      [dict(row, task_id="TASK-999")], [dict(row, task_id=None)],
                      [{"operation": "add", "field": "title", "after": "Duplicate"}],
                      [dict(replace, after=task["title"])], [dict(replace, operation="guess")]):
            request = self.request()
            request["edits"] = edits
            before = self.fixture.fixture.snapshot()
            with self.subTest(edits=edits), self.assertRaises(WorkError):
                self.prepare(request)
            self.assertEqual(before, self.fixture.fixture.snapshot())

    def test_missing_decisions_conflicting_input_and_identity_changes_are_rejected(self):
        candidates = []
        request = self.request()
        request["decisions"] = []
        candidates.append(request)
        request = self.request()
        request.update(task=self.fixture.fixture.contract, edits=[])
        candidates.append(request)
        request = self.request()
        request["edits"] = [{"operation": "replace", "field": "requirement_id", "before": "example", "after": "other"}]
        candidates.append(request)
        for request in candidates:
            with self.subTest(request=request), self.assertRaises(WorkError):
                self.prepare(request)

    def test_complete_does_not_invent_missing_content_or_refresh_sources(self):
        for field in ("summary", "source_plan"):
            task = copy.deepcopy(self.fixture.fixture.contract)
            if field == "summary":
                del task[field]
            else:
                task[field]["canonical_sha256"] = "0" * 64
            self.fixture.task_path.write_text(json.dumps(task))
            before = self.fixture.fixture.snapshot()
            with self.subTest(field=field), self.assertRaises(WorkError):
                self.prepare(self.request("complete"))
            self.assertEqual(before, self.fixture.fixture.snapshot())

    def test_sources_changed_during_or_after_preview_block_output(self):
        real = task_repair_prepare.repair_task
        output = self.root / "prepared.json"
        for after_preview in (False, True):
            self.fixture.task_path.write_bytes(b"\xef\xbb\xbf" + self.fixture.fixture.raw)
            def change(raw, **options):
                result = real(raw, **options) if after_preview else None
                self.fixture.task_path.write_bytes(self.fixture.task_path.read_bytes() + b"\n")
                return result if after_preview else real(raw, **options)
            with self.subTest(after_preview=after_preview), patch.object(task_repair_prepare, "repair_task", side_effect=change):
                with self.assertRaises(WorkError) as error:
                    self.prepare(self.request(), output_file=str(output))
                self.assertEqual(error.exception.code, "task_repair_source_changed")
            self.assertFalse(output.exists())

    def test_writer_lock_and_unfinished_transaction_block_preparation(self):
        with state_writer(self.root, self.fixture.artifacts["execution"]):
            with self.assertRaises(WorkError):
                self.prepare(self.request())
        self.fixture.fixture.index["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-001"}
        self.fixture.fixture.save_index()
        with self.assertRaises(WorkError) as error:
            self.prepare(self.request())
        self.assertEqual(error.exception.code, "task_repair_diagnose_only")
        del self.fixture.fixture.index["lock"]
        self.fixture.fixture.save_index()
        (self.fixture.fixture.directory / ".work-task-repair-other.json").write_bytes(b"{}")
        with self.assertRaises(WorkError):
            self.prepare(self.request())

    def test_cli_output_is_request_only_and_exclusive(self):
        output = self.root / "prepared.json"
        args = self.input_arguments(["--project-root", str(self.root), "task", "repair-prepare",
            "--input-file", "request.json", "--user-config-root", str(self.root), "--output-file", str(output)],
            b"\xef\xbb\xbf" + json.dumps(self.request()).encode())
        stdout = io.StringIO()
        self.assertEqual(main(args, stdout=stdout), 0, stdout.getvalue())
        data = json.loads(stdout.getvalue())["data"]
        self.assertEqual(json.loads(output.read_bytes()), data["request"])
        self.assertEqual(self.fixture.run_repair(json.loads(output.read_bytes())), data["preview"])
        before = self.fixture.fixture.snapshot()
        with self.assertRaises(FileExistsError):
            self.prepare(self.request(), output_file=str(output))
        self.assertEqual(before, self.fixture.fixture.snapshot())
