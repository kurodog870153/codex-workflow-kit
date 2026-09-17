from __future__ import annotations
import json, sys, unittest
from pathlib import Path
SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
from tests.work.contracts import test_task_diagnostics as fixtures
from worklib.artifacts.layout_migration import layout_migration
from worklib.foundation import spec_transactions
from worklib.foundation.errors import WorkError

class LayoutMigrationFlowTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.TaskDiagnosticsTests("test_valid_report_and_cli_are_read_only")
        self.fixture.setUp(); self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.plan_path = self.fixture.artifacts["plan"]
    def call(self, schema, operation, **extra):
        raw = json.dumps({"schema": schema, "plan_path": self.plan_path, **extra}).encode()
        return layout_migration(raw, project_root=self.root, user_config_root=str(self.root), operation=operation,
                                approved_sha256=extra.get("approved_sha256"))
    def test_full_flow_preserves_v1_task(self):
        old = (self.root / self.fixture.artifacts["task"]).read_bytes()
        self.assertTrue(self.call("work-task-layout-preflight-request/v1", "preflight")["can_prepare_candidate"])
        prepared = self.call("work-task-layout-preflight-request/v1", "prepare")
        request = prepared["request"]
        preview = layout_migration(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root), operation="validate")
        applied = layout_migration(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root), operation="apply", approved_sha256=preview["approved_sha256"])
        self.assertEqual(applied["status"], "migrated")
        self.assertEqual((self.root / self.fixture.artifacts["task"]).read_bytes(), old)
        self.assertTrue((self.root / applied["target_task_path"]).is_file())

    def test_active_execution_and_pending_transaction_are_blocked(self):
        self.fixture.index["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-001"}
        self.fixture.save_index()
        with self.assertRaises(WorkError) as caught:
            self.call("work-task-layout-preflight-request/v1", "preflight")
        self.assertEqual(caught.exception.code, "layout_migration_active_execution")
        del self.fixture.index["lock"]
        self.fixture.save_index()
        (self.fixture.directory / ".work-spec-update-pending.json").write_bytes(b"{}")
        with self.assertRaises(WorkError):
            self.call("work-task-layout-preflight-request/v1", "preflight")

    def test_interruption_recovery_verification_and_history_preservation(self):
        history = self.fixture.directory / "TASK-001" / "ATTEMPT-001" / "evidence.bin"
        history.parent.mkdir(parents=True)
        history.write_bytes(b"history")
        prepared = self.call("work-task-layout-preflight-request/v1", "prepare")
        request = prepared["request"]
        preview = layout_migration(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root), operation="validate")
        real = spec_transactions.publish_journal
        with unittest.mock.patch.object(spec_transactions, "publish_journal", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                layout_migration(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root), operation="apply", approved_sha256=preview["approved_sha256"])
        recovered = layout_migration(json.dumps(request).encode(), project_root=self.root, user_config_root=str(self.root), operation="recover", approved_sha256=preview["approved_sha256"])
        self.assertEqual(recovered["status"], "recovered")
        self.assertEqual(history.read_bytes(), b"history")
        verification = {"schema": "work-task-layout-verify-request/v1", "plan_path": self.plan_path, "record": recovered["record"]}
        report = layout_migration(json.dumps(verification).encode(), project_root=self.root, user_config_root=str(self.root), operation="verify")
        self.assertTrue(report["verified"])
        history.write_bytes(b"changed history")
        changed = layout_migration(json.dumps(verification).encode(), project_root=self.root, user_config_root=str(self.root), operation="verify")
        self.assertFalse(changed["verified"])

if __name__ == "__main__": unittest.main()
