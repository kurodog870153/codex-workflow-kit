import copy
import json
import sys
from pathlib import Path

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills" / "work" / "scripts"))
from cli_support import FileInputTestCase
from artifacts import test_specification as fixtures
from worklib.artifacts.migration_transactions import verification_selection
from worklib.foundation.errors import WorkError


class MigrationTransactionTests(FileInputTestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.request = self.fixture.migration_request()
        self.complete(self.request)

    def complete(self, request):
        preview = self.fixture.run_migration(request)
        return self.fixture.run_migration(request, operation="apply", approval=preview["approved_sha256"])

    def rejected(self, request, code):
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError) as caught:
            self.fixture.run_migration(request)
        self.assertEqual(caught.exception.code, code)
        self.assertEqual(self.fixture.snapshot(), before)

    def selection(self):
        return verification_selection(self.fixture.root, self.fixture.artifacts["execution"],
                                      {key: path.read_bytes() for key, path in self.fixture.paths().items()})

    def test_same_request_and_conflicting_request(self):
        self.rejected(self.request, "migration_already_completed")
        changed = copy.deepcopy(self.request)
        changed["instruction_review"]["plan"] += " changed"
        self.rejected(changed, "migration_record_conflict")

    def test_sources_cannot_be_reused_under_new_id(self):
        changed = copy.deepcopy(self.request)
        changed["task"]["spec_id"] = "TASK-SPEC-003"
        self.rejected(changed, "migration_source_already_used")

    def test_missing_and_invalid_marker_require_recovery(self):
        directory = self.fixture.root / self.fixture.artifacts["execution"]
        marker = next(directory.glob(".work-spec-update-*.json.done"))
        original = marker.read_bytes()
        marker.unlink()
        self.rejected(self.request, "spec_update_pending")
        marker.write_bytes(b"bad\n")
        self.rejected(self.request, "spec_update_pending")
        marker.write_bytes(original)

    def test_orphan_marker_is_rejected(self):
        marker = self.fixture.root / self.fixture.artifacts["execution"] / ".work-spec-update-SPEC-UPDATE-999.json.done"
        marker.write_bytes(b"orphan\n")
        self.rejected(self.request, "migration_orphan_marker")

    def test_old_and_new_records_select_current_bytes(self):
        self.assertEqual(self.selection()["applicable_record_id"], "SPEC-UPDATE-002")
        newer = self.fixture.request()
        newer["schema"] = "work-spec-migration-request/v1"
        newer["instruction_review"] = self.request["instruction_review"]
        self.complete(newer)
        selection = self.selection()
        self.assertEqual(selection["latest_completed_record_id"], "SPEC-UPDATE-003")
        self.assertEqual(selection["applicable_record_id"], "SPEC-UPDATE-003")
        self.rejected(self.request, "migration_already_completed")
        directory = self.fixture.root / self.fixture.artifacts["execution"]
        self.assertEqual(len(list(directory.glob(".work-spec-update-*.json.done"))), 2)
        path = self.fixture.paths()["index"]
        path.write_bytes(path.read_bytes() + b"\n")
        self.assertIsNone(self.selection()["applicable_record_id"])
        self.assertEqual(self.selection()["latest_completed_record_id"], "SPEC-UPDATE-003")
