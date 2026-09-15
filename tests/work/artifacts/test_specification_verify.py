from __future__ import annotations

import copy
import io
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from artifacts import test_specification as fixtures
from worklib.artifacts.specification_verify import verify_specification
from worklib.cli import main
from worklib.foundation.errors import ExitCode


class SpecificationVerifyTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root

    def publish(self):
        request = self.fixture.request()
        preview = self.fixture.run_update(request)
        published = self.fixture.run_update(request, "apply", preview["approved_sha256"])
        self.request = published["verification_request"]
        self.journal = self.root / self.fixture.artifacts["execution"] / (
            ".work-spec-update-" + preview["record_id"] + ".json")
        self.marker = self.journal.with_name(self.journal.name + ".done")
        return published

    def verify(self):
        before = self.fixture.snapshot()
        result = verify_specification(
            json.dumps(self.request).encode("utf-8"), project_root=self.root,
            user_config_root=str(self.root),
        )
        self.assertEqual(self.fixture.snapshot(), before)
        return result

    def test_success_checks_exact_ordinary_update_without_writes(self):
        published = self.publish()
        self.assertEqual(published["next_step"], {
            "command": "task spec-verify", "input": "verification_request",
        })
        result = self.verify()
        self.assertTrue(result["verified"])
        self.assertEqual(result["status"], "verified")
        self.assertEqual(result["verification_scope"], "exact_specification_result")
        self.assertFalse(result["execution_authorized"])
        self.assertEqual(result["next_step"], "normal_execute_preflight")

    def test_missing_or_wrong_completion_marker_blocks_verification(self):
        self.publish()
        self.marker.write_bytes(b"bad marker")
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "spec_verify_completion_mismatch" for issue in result["issues"]))
        self.marker.unlink()
        self.assertFalse(self.verify()["verified"])

    def test_changed_installed_artifact_or_history_is_detected(self):
        for target in ("task", "history"):
            with self.subTest(target=target):
                self.fixture.doCleanups()
                self.setUp()
                self.publish()
                if target == "task":
                    self.fixture.task_path.write_bytes(b"changed")
                else:
                    path = self.fixture.index_path.parent / "TASK-001/extra.txt"
                    path.parent.mkdir(parents=True)
                    path.write_text("changed")
                self.assertFalse(self.verify()["verified"])

    def test_migration_journal_is_not_an_ordinary_specification(self):
        request = self.fixture.migration_request()
        preview = self.fixture.run_migration(request)
        published = self.fixture.run_migration(request, "apply", preview["approved_sha256"])
        self.request = copy.deepcopy(published["verification_request"])
        self.request["schema"] = "work-spec-verification-request/v1"
        result = self.verify()
        self.assertFalse(result["verified"])
        self.assertTrue(any(issue["code"] == "spec_verify_not_specification" for issue in result["issues"]))

    def test_cli_success_and_failure_return_structured_stdout(self):
        self.publish()
        for blocked in (False, True):
            if blocked:
                self.marker.write_bytes(b"bad marker")
            output, error = io.StringIO(), io.StringIO()
            code = main(self.fixture.input_arguments([
                "--project-root", str(self.root), "task", "spec-verify", "--input-file", "request.json",
                "--user-config-root", str(self.root),
            ], json.dumps(self.request)), stdout=output, stderr=error)
            self.assertEqual(code, ExitCode.ARTIFACT_INTEGRITY if blocked else ExitCode.SUCCESS)
            self.assertEqual(error.getvalue(), "")
            response = json.loads(output.getvalue())
            self.assertEqual(response["data"]["schema"], "work-spec-verification/v1")
            self.assertEqual(response["reason_code"], "specification_verification_failed" if blocked else "ok")


if __name__ == "__main__":
    unittest.main()
