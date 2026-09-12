from __future__ import annotations

import hashlib
import io
import json
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills" / "work" / "scripts"))
from cli_support import FileInputTestCase
from artifacts import test_specification as fixtures
from worklib.artifacts.migration_preflight import migration_preflight
from worklib.cli import main
from worklib.contracts.execution_index import render_execution_index
from worklib.foundation.spec_update import state_writer


class MigrationPreflightTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.request = {
            "schema": "work-migration-preflight-request/v1",
            "requirement_id": "example", "artifacts": self.fixture.artifacts,
        }

    def run_preflight(self):
        before = self.fixture.snapshot()
        result = migration_preflight(
            json.dumps(self.request).encode("utf-8"), project_root=self.root, user_config_root=str(self.root),
        )
        self.assertEqual(self.fixture.snapshot(), before)
        return result

    def test_current_contract_requires_no_automatic_revision(self):
        result = self.run_preflight()
        self.assertEqual(result["status"], "current")
        self.assertTrue(result["can_prepare_candidate"])
        self.assertEqual(result["task_spec_id"], "TASK-SPEC-001")
        self.assertNotIn("candidate", result)
        self.assertNotIn("approved_sha256", result)

    def test_work_source_drift_is_reviewable_and_lists_changed_sources(self):
        self.fixture.migration_request()
        result = self.run_preflight()
        self.assertEqual(result["status"], "review_required")
        self.assertFalse(result["current_diagnostics"]["normal_use_allowed"])
        self.assertTrue(result["historical_diagnostics"]["normal_use_allowed"])
        self.assertTrue(result["instruction_sources"]["plan"]["changed_sources"])
        self.assertFalse(result["historical_content_verified"])
        self.assertTrue(result["execute_instruction_selections"])

    def test_binding_mismatch_requires_decision_without_auto_acceptance(self):
        self.fixture.migration_repair_request()
        result = self.run_preflight()
        self.assertEqual(result["status"], "blocked")
        self.assertTrue(any(item["kind"] == "source_plan_baseline" for item in result["required_decisions"]))

    def test_multiple_faults_are_reported_and_dependencies_skipped(self):
        self.fixture.task_path.write_bytes(b"{")
        index = json.loads(self.fixture.index_path.read_bytes())
        index["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-002"}
        self.fixture.index_path.write_bytes(render_execution_index(index))
        result = self.run_preflight()
        self.assertFalse(result["can_prepare_candidate"])
        self.assertEqual(result["historical_diagnostics"]["structure_status"], "not_checked")
        self.assertTrue(any(item["code"] == "task_repair_execution_lock"
                            for item in result["historical_diagnostics"]["issues"]))
        self.assertTrue(any(item["code"] == "invalid_json_contract" for item in result["issues"]))

    def test_invalid_encoding_and_missing_files_are_structured(self):
        self.fixture.task_path.write_bytes(b"\xff")
        self.fixture.plan_path.unlink()
        result = self.run_preflight()
        self.assertFalse(result["can_prepare_candidate"])
        self.assertIsNone(result["fingerprints"]["plan"]["raw_sha256"])
        self.assertEqual(result["fingerprints"]["task"]["raw_sha256"], hashlib.sha256(b"\xff").hexdigest())

    def test_completed_migration_is_listed_without_rerun(self):
        request = self.fixture.migration_request()
        preview = self.fixture.run_migration(request)
        self.fixture.run_migration(request, "apply", preview["approved_sha256"])
        result = self.run_preflight()
        migrations = [item for item in result["transactions"] if item["kind"] == "migration"]
        self.assertEqual(len(migrations), 1)
        self.assertEqual(migrations[0]["completion"], "completed")
        self.assertTrue(result["can_prepare_candidate"])

    def test_pending_or_corrupt_completed_transaction_blocks(self):
        directory = self.root / self.fixture.artifacts["execution"]
        path = directory / ".work-spec-update-SPEC-UPDATE-002.json"
        path.write_bytes(b"{}")
        marker = path.with_name(path.name + ".done")
        marker.write_bytes(hashlib.sha256(b"{}").hexdigest().encode("ascii") + b"\n")
        result = self.run_preflight()
        self.assertFalse(result["can_prepare_candidate"])
        self.assertEqual(result["transactions"][0]["completion"], "unreadable")
        marker.unlink()
        path.write_text(json.dumps({"schema": "work-spec-update-record/v1",
                                    "request": {"schema": "work-spec-migration-request/v1"}}), encoding="utf-8")
        self.assertEqual(self.run_preflight()["transactions"][0]["completion"], "incomplete")

    def test_busy_writer_blocks_without_modifying_mutex(self):
        with state_writer(self.root, self.fixture.artifacts["execution"]):
            result = self.run_preflight()
        self.assertFalse(result["can_prepare_candidate"])
        self.assertTrue(any(item["code"] == "work_state_writer_busy" for item in result["issues"]))

    def test_non_git_workspace_reports_visibility_unavailable(self):
        self.assertEqual(self.run_preflight()["git"]["status"], "not_checked")

    def test_git_visibility_tracks_tracked_ignored_and_untracked_artifacts(self):
        def git(*args):
            subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True, timeout=10)
        git("init")
        git("add", "--", self.fixture.artifacts["plan"])
        (self.root / ".gitignore").write_text(self.fixture.artifacts["execution"] + "/\n", encoding="utf-8")
        result = self.run_preflight()["git"]
        self.assertEqual(result["status"], "checked")
        self.assertEqual(result["artifacts"]["plan"]["visibility"], "tracked")
        self.assertEqual(result["artifacts"]["task"]["visibility"], "untracked")
        self.assertEqual(result["artifacts"]["index"]["visibility"], "ignored")

    def test_git_failure_does_not_hide_contract_diagnostics(self):
        with patch("worklib.artifacts.migration_preflight.subprocess.run", side_effect=FileNotFoundError):
            result = self.run_preflight()
        self.assertEqual(result["git"]["status"], "not_checked")
        self.assertTrue(result["can_prepare_candidate"])

    def test_cli_accepts_bom_file_and_returns_json_when_blocked(self):
        for blocked in (False, True):
            if blocked:
                self.fixture.task_path.write_bytes(b"{")
            output, errors = io.StringIO(), io.StringIO()
            code = main([
                "--project-root", str(self.root), "task", "migrate-preflight",
                "--input-file", self.input_file(b"\xef\xbb\xbf" + json.dumps(self.request).encode("utf-8")),
                "--user-config-root", str(self.root),
            ], stdout=output, stderr=errors)
            self.assertEqual(code, 4 if blocked else 0)
            self.assertEqual(errors.getvalue(), "")
            envelope = json.loads(output.getvalue())
            self.assertEqual(envelope["data"]["schema"], "work-migration-preflight/v1")
            self.assertEqual(envelope["reason_code"], "migration_preflight_blocked" if blocked else "ok")
