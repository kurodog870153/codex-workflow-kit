from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT = REPO_ROOT / "skills" / "work" / "scripts" / "work.py"
TEST_ROOT = REPO_ROOT / "tests" / "work"
sys.path.insert(0, str(TEST_ROOT))

from artifacts import test_specification as fixtures


class MigrationFlowIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.request_directory = tempfile.TemporaryDirectory(prefix="migration-flow-")
        self.addCleanup(self.request_directory.cleanup)
        self.request_root = Path(self.request_directory.name)

    def run_cli(self, command: str, request: dict, *, expected: int, approval: str | None = None):
        request_path = self.request_root / (command + ".json")
        request_path.write_text(json.dumps(request, ensure_ascii=False), encoding="utf-8")
        arguments = [
                sys.executable,
                "-B",
                str(SCRIPT),
                "--project-root",
                str(self.fixture.root),
                "task",
                command,
                "--input-file",
                str(request_path),
                "--user-config-root",
                str(self.fixture.root),
            ]
        if approval is not None:
            arguments.extend(["--approved-sha256", approval])
        result = subprocess.run(
            arguments,
            cwd=REPO_ROOT,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            shell=False,
            timeout=30,
        )
        self.assertEqual(result.returncode, expected, result.stderr.decode("utf-8"))
        self.assertEqual(result.stderr, b"")
        self.assertNotIn(b"\r\n", result.stdout)
        response = json.loads(result.stdout.decode("utf-8"))
        self.assertEqual(response["schema"], "work-cli-result/v1")
        self.assertEqual(set(response), {"schema", "status", "reason_code", "message", "data"})
        return response

    def test_preflight_validate_apply_verify_end_to_end(self):
        migration = self.fixture.migration_request()
        preflight = {
            "schema": "work-migration-preflight-request/v1",
            "requirement_id": "example",
            "artifacts": self.fixture.artifacts,
        }
        result = self.run_cli("migrate-preflight", preflight, expected=0)
        self.assertEqual(result["data"]["status"], "review_required")
        self.assertTrue(result["data"]["can_prepare_candidate"])

        result = self.run_cli("migrate-validate", migration, expected=0)
        preview = result["data"]
        self.assertEqual(preview["status"], "valid")
        self.assertEqual(preview["file_readiness"], "requires_execute_preflight")
        approval = preview["approved_sha256"]

        result = self.run_cli("migrate", migration | {}, expected=0, approval=approval)
        self.assertEqual(result["data"]["status"], "updated")
        record_id = result["data"]["record_id"]
        approval = preview["approved_sha256"]
        self.assertEqual(record_id, preview["record_id"])

        verification = {
            "schema": "work-migration-verify-request/v1",
            "requirement_id": "example",
            "artifacts": self.fixture.artifacts,
            "record_id": record_id,
        }
        result = self.run_cli("migrate-verify", verification, expected=0)
        self.assertTrue(result["data"]["verified"])
        self.assertFalse(result["data"]["execution_authorized"])

        result = self.run_cli("migrate-verify", verification, expected=0)
        self.assertTrue(result["data"]["verified"])
        self.assertEqual(result["status"], "success")
        self.assertEqual(approval, preview["approved_sha256"])

    def test_apply_with_changed_request_returns_structured_failure_without_write(self):
        migration = self.fixture.migration_request()
        preview = self.run_cli("migrate-validate", migration, expected=0)["data"]
        changed = json.loads(json.dumps(migration))
        changed["reason"] = "Changed after review."
        before = self.fixture.snapshot()
        result = self.run_cli("migrate", changed, expected=5, approval=preview["approved_sha256"])
        self.assertEqual(result["status"], "rejected")
        self.assertIn(result["reason_code"], {"spec_update_approval_changed", "spec_update_change_evidence"})
        self.assertEqual(self.fixture.snapshot(), before)
        self.assertEqual(preview["status"], "valid")

    def test_completed_transaction_cannot_be_applied_twice(self):
        migration = self.fixture.migration_request()
        preview = self.run_cli("migrate-validate", migration, expected=0)["data"]
        self.run_cli("migrate", migration | {}, expected=0, approval=preview["approved_sha256"])
        before = self.fixture.snapshot()
        result = self.run_cli("migrate", migration | {}, expected=5, approval=preview["approved_sha256"])
        self.assertEqual(result["status"], "rejected")
        self.assertEqual(result["reason_code"], "migration_already_completed")
        self.assertEqual(self.fixture.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
