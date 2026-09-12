from __future__ import annotations

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
from artifacts import test_specification as fixtures
from contracts import test_attempt as attempt_fixtures
from worklib.artifacts import migration_verify
from worklib.artifacts.specification import _hash, _json
from worklib.cli import main
from worklib.contracts.attempt import canonicalize_attempt_contract
from worklib.contracts.execution_index import render_execution_index
from worklib.foundation.errors import WorkError
from worklib.foundation.markdown import render_json_contract
from worklib.foundation.spec_update import state_writer


class MigrationVerifyTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root

    def migrate(self):
        request = self.fixture.migration_request()
        preview = self.fixture.run_migration(request)
        self.fixture.run_migration(request, "apply", preview["approved_sha256"])
        self.request = {
            "schema": "work-migration-verify-request/v1", "requirement_id": "example",
            "artifacts": self.fixture.artifacts, "record_id": preview["record_id"],
        }
        self.journal = self.root / self.fixture.artifacts["execution"] / (".work-spec-update-" + preview["record_id"] + ".json")
        self.marker = self.journal.with_name(self.journal.name + ".done")

    def verify(self):
        before = self.fixture.snapshot()
        result = migration_verify.verify_migration(
            json.dumps(self.request).encode("utf-8"), project_root=self.root, user_config_root=str(self.root),
        )
        self.assertEqual(self.fixture.snapshot(), before)
        return result

    def test_success_checks_exact_result_without_writes(self):
        self.migrate()
        with patch("worklib.artifacts.specification._write") as write:
            result = self.verify()
        write.assert_not_called()
        self.assertTrue(result["verified"])
        self.assertEqual(result["status"], "verified")
        self.assertEqual(result["completion_status"], "passed")
        self.assertEqual(result["verification_scope"], "exact_migration_result")
        self.assertFalse(result["execution_authorized"])
        self.assertTrue(result["task_diagnostics"]["normal_use_allowed"])
        self.assertIn("migration_evidence", [check["name"] for check in result["checks"]])

    def migrate_with_binding_repair(self):
        request = self.fixture.migration_repair_request()
        preview = self.fixture.run_migration(request)
        self.fixture.run_migration(request, "apply", preview["approved_sha256"])
        self.request = {
            "schema": "work-migration-verify-request/v1", "requirement_id": "example",
            "artifacts": self.fixture.artifacts, "record_id": preview["record_id"],
        }
        self.journal = self.root / self.fixture.artifacts["execution"] / (
            ".work-spec-update-" + self.request["record_id"] + ".json")
        self.marker = self.journal.with_name(self.journal.name + ".done")

    def test_reviewed_binding_repair_migration_verifies(self):
        self.migrate_with_binding_repair()
        self.assertTrue(self.verify()["verified"])

    def test_missing_or_wrong_marker_is_not_success(self):
        self.migrate()
        self.marker.write_bytes(b"0" * 64 + b"\n")
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_completion_mismatch" for issue in result["issues"]))
        self.marker.unlink()
        result = self.verify()
        self.assertEqual(result["completion_status"], "not_checked")
        self.assertFalse(result["verified"])

    def test_corrupt_journal_still_returns_document_diagnostics(self):
        self.migrate()
        self.journal.write_bytes(b"{")
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertEqual(result["task_diagnostics"]["contract_status"], "passed")
        self.assertTrue(any(check["name"] == "migration_evidence" and check["status"] == "not_checked" for check in result["checks"]))

    def test_matching_marker_cannot_hide_inconsistent_derived_evidence(self):
        self.migrate()
        record = json.loads(self.journal.read_bytes())
        record["affected_task_ids"] = []
        raw = _json(record)
        self.journal.write_bytes(raw)
        self.marker.write_bytes(_hash(raw).encode("ascii") + b"\n")
        result = self.verify()
        self.assertEqual(result["completion_status"], "passed")
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_record_mismatch" for issue in result["issues"]))

    def test_later_artifact_edits_are_reported_without_rollback(self):
        self.migrate()
        task = json.loads(self.fixture.task_path.read_bytes())
        task["summary"] = "Later edit."
        from worklib.contracts.task import render_task_contract
        self.fixture.task_path.write_bytes(render_task_contract(task))
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_state_changed" for issue in result["issues"]))

    def assert_record_rejected(self, record):
        raw = _json(record)
        self.journal.write_bytes(raw)
        self.marker.write_bytes(_hash(raw).encode("ascii") + b"\n")
        result = self.verify()
        self.assertEqual(result["completion_status"], "passed")
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_record_mismatch" for issue in result["issues"]))

    def test_request_candidates_must_match_installed_content(self):
        self.migrate()
        original = json.loads(self.journal.read_bytes())
        for artifact in ("plan", "task"):
            with self.subTest(artifact=artifact):
                record = copy.deepcopy(original)
                record["request"][artifact]["summary"] = "Different approved content."
                self.assert_record_rejected(record)

    def test_migration_evidence_is_fully_revalidated(self):
        self.migrate()
        original = json.loads(self.journal.read_bytes())
        replacements = {
            "plan_edits": [],
            "execute_instruction_selections": {},
            "source_plan_repair": {"review": "Unapproved repair"},
        }
        for field, value in replacements.items():
            with self.subTest(field=field):
                record = copy.deepcopy(original)
                record["migration"][field] = value
                self.assert_record_rejected(record)

    def test_reviewed_binding_evidence_cannot_be_removed_or_changed(self):
        self.migrate_with_binding_repair()
        original = json.loads(self.journal.read_bytes())
        for remove in (True, False):
            with self.subTest(remove=remove):
                record = copy.deepcopy(original)
                if remove:
                    del record["migration"]["source_plan_repair"]
                else:
                    record["migration"]["source_plan_repair"] = {}
                self.assert_record_rejected(record)

    def test_missing_target_record_is_reported_without_fallback_to_latest(self):
        self.migrate()
        self.request["record_id"] = "SPEC-UPDATE-999"
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertIsNone(result["journal_sha256"])

    def test_record_id_cannot_escape_execution_directory(self):
        self.migrate()
        self.request["record_id"] = "../outside"
        with self.assertRaises(WorkError) as error:
            self.verify()
        self.assertEqual(error.exception.code, "migration_verify_record_id")

    def add_attempt(self):
        fixture = attempt_fixtures.AttemptContractTests()
        fixture.setUp()
        attempt = copy.deepcopy(fixture.attempt)
        attempt.update(status="stopped", final_type="instructions_changed", reason="Historical stop.",
                       ended_at="2026-09-01T10:05+08:00")
        path = self.root / self.fixture.artifacts["execution"] / "TASK-001/ATTEMPT-001/attempt.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(render_json_contract(canonicalize_attempt_contract(attempt, project_root=self.root)))
        return path

    def test_historical_attempts_keep_old_fingerprints_and_validate(self):
        path = self.add_attempt()
        before = path.read_bytes()
        self.migrate()
        result = self.verify()
        self.assertTrue(result["verified"])
        self.assertEqual(path.read_bytes(), before)
        self.assertIn(path.relative_to(self.root).as_posix(), result["history_validation"])

    def test_correction_contracts_are_verified_without_changing_history(self):
        from worklib.contracts.correction import render_correction_contract
        attempt = self.add_attempt()
        correction = {
            "schema": "work-correction/v1", "correction_id": "ATTEMPT-001-CORRECTION-001",
            "created_at": "2026-09-01T10:06+08:00", "target_attempt_id": "ATTEMPT-001",
            "task_instructions_sha256": "b" * 64, "execute_instructions_sha256": "c" * 64,
            "field": "reason", "correct_value": "Reviewed historical stop.", "reason": "Clarified the recorded explanation.",
        }
        path = attempt.parent / "corrections" / "ATTEMPT-001-CORRECTION-001.json"
        path.parent.mkdir()
        path.write_bytes(render_correction_contract(correction))
        self.migrate()
        result = self.verify()
        self.assertTrue(result["verified"])
        self.assertIn(path.relative_to(self.root).as_posix(), result["history_validation"])
        path.write_bytes(b"{")
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(check["name"] == "history_contract:" + path.relative_to(self.root).as_posix()
                            and check["status"] == "failed" for check in result["checks"]))

    def test_deleted_history_is_detected_independently(self):
        path = self.add_attempt()
        self.migrate()
        path.unlink()
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_history_changed" for issue in result["issues"]))

    def test_index_reference_to_missing_attempt_is_reported(self):
        self.migrate()
        index = json.loads(self.fixture.index_path.read_bytes())
        index["tasks"][0].update(status="pending_retry", latest_attempt="ATTEMPT-009",
                                  status_reason={"kind": "task_change", "ref": "TASK-CHANGE-001"})
        self.fixture.index_path.write_bytes(render_execution_index(index))
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_attempt_reference" for issue in result["issues"]))

    def test_active_writer_and_unrelated_pending_transaction_block(self):
        self.migrate()
        with state_writer(self.root, self.fixture.artifacts["execution"]):
            self.assertFalse(self.verify()["verified"])
        other = self.journal.with_name(".work-task-repair-other.json")
        other.write_bytes(b"{")
        self.assertFalse(self.verify()["verified"])

    def test_git_failure_is_informational(self):
        self.migrate()
        with patch("worklib.artifacts.migration_preflight.subprocess.run", side_effect=FileNotFoundError):
            result = self.verify()
        self.assertTrue(result["verified"])
        self.assertEqual(result["git"]["status"], "not_checked")

    def test_concurrent_change_during_git_inspection_is_detected(self):
        self.migrate()
        def changed(*args):
            self.fixture.task_path.write_bytes(b"external change")
            return {"status": "not_checked", "artifacts": {}}
        with patch.object(migration_verify, "_git_visibility", side_effect=changed):
            result = migration_verify.verify_migration(
                json.dumps(self.request).encode("utf-8"), project_root=self.root, user_config_root=str(self.root),
            )
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "migration_verify_concurrent_change" for issue in result["issues"]))

    def test_cli_success_and_failure_always_return_json_stdout(self):
        self.migrate()
        for blocked in (False, True):
            if blocked:
                self.marker.write_bytes(b"bad marker")
            output, errors = io.StringIO(), io.StringIO()
            code = main([
                "--project-root", str(self.root), "task", "migrate-verify",
                "--input-file", self.input_file(b"\xef\xbb\xbf" + json.dumps(self.request).encode("utf-8")),
                "--user-config-root", str(self.root),
            ], stdout=output, stderr=errors)
            self.assertEqual(code, 5 if blocked else 0)
            self.assertEqual(errors.getvalue(), "")
            envelope = json.loads(output.getvalue())
            self.assertEqual(envelope["data"]["schema"], "work-migration-verification/v1")
            self.assertEqual(envelope["reason_code"], "migration_verification_failed" if blocked else "ok")
