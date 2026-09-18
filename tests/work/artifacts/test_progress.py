from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_progress as fixtures
from worklib.business_services.progress import prepare_progress as _prepare_progress
from worklib.business_services.progress import preview_progress as _preview_progress
from worklib.business_services.progress import read_progress, save_progress as _save_progress
from worklib.foundation.markdown import render_json_contract

def preview_progress(root, value, *, expected_revision):
    return _preview_progress(root, render_json_contract(value), source="test", expected_revision=expected_revision)

def prepare_progress(root, value, **kwargs):
    return _prepare_progress(root, render_json_contract(value), source="test", **kwargs)

def save_progress(root, value, **kwargs):
    return _save_progress(root, render_json_contract(value), source="test", **kwargs)
from worklib.foundation.errors import WorkError


class ProgressStorageTests(unittest.TestCase):
    def content(self):
        return {key: copy.deepcopy(value) for key, value in self.value.items()
                if key not in {"schema", "requirement_id", "mode", "revision", "status"}}

    def prepare(self, content=None, revision=0, mode="task"):
        return prepare_progress(self.root, self.content() if content is None else content,
                                requirement_id="example", mode=mode, expected_revision=revision)

    def test_prepare_preserves_content_and_matches_existing_approval(self):
        content = self.content()
        original = copy.deepcopy(content)
        result = self.prepare(content)
        self.assertEqual(result["progress"], self.value)
        self.assertEqual(result["approved_sha256"],
                         preview_progress(self.root, self.value, expected_revision=0)["approved_sha256"])
        self.assertEqual(list(self.root.iterdir()), [])
        result["progress"]["notes"].append("Caller edit")
        self.assertEqual(content, original)
        self.assertEqual(result["source_validation"], "not_checked")
        self.assertEqual(result["evidence_trust"], "historical_context_only")
        self.assertEqual(result["formal_readiness"], "not_established")

    def test_formal_looking_context_remains_unverified_historical_evidence(self):
        content = self.content()
        content["context"] = {
            "plan_sha256": "a" * 64, "task_raw_sha256": "b" * 64,
            "status": "valid", "approved_sha256": "c" * 64,
        }
        prepared = self.prepare(content)
        validated = preview_progress(self.root, prepared["progress"], expected_revision=0)
        for result in (prepared, validated):
            self.assertEqual(result["source_validation"], "not_checked")
            self.assertEqual(result["evidence_trust"], "historical_context_only")
            self.assertEqual(result["formal_readiness"], "not_established")
        self.assertEqual(prepared["progress"]["context"], content["context"])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_prepare_second_revision_saves_through_existing_api(self):
        first = self.save()
        original = (self.root / first["path"]).read_bytes()
        content = self.content()
        content["notes"] = ["Complete replacement discussion"]
        result = self.prepare(content, revision=1)
        self.assertEqual(result["progress"]["revision"], 2)
        self.assertEqual(result["progress"]["notes"], content["notes"])
        save_progress(self.root, result["progress"], expected_revision=1,
                      approved_sha256=result["approved_sha256"])
        self.assertEqual(read_progress(self.root, "example", "task")["progress"], result["progress"])
        history = self.root / Path(first["path"]).parent / "history/1/progress.json"
        self.assertEqual(history.read_bytes(), original)

    def test_prepare_rejects_machine_overrides_missing_content_and_invalid_revision(self):
        for field in ("schema", "requirement_id", "mode", "revision", "status"):
            with self.subTest(field=field):
                content = self.content()
                content[field] = self.value[field]
                with self.assertRaises(WorkError):
                    self.prepare(content)
        content = self.content()
        del content["tentative"]
        with self.assertRaises(WorkError):
            self.prepare(content)
        for revision in (True, -1, "0"):
            with self.subTest(revision=revision), self.assertRaises(WorkError) as caught:
                self.prepare(revision=revision)
            self.assertEqual(caught.exception.code, "invalid_progress_revision")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_prepare_keeps_plan_task_identity_boundary(self):
        with self.assertRaises(WorkError) as caught:
            self.prepare(mode="plan")
        self.assertEqual(caught.exception.code, "invalid_progress_task")
        content = self.content()
        content["current_task_id"] = None
        result = self.prepare(content, mode="plan")
        self.assertEqual(result["path"], "outputs/work/progress/example/plan/progress.json")
        self.assertEqual(result["progress"]["source_status"], content["source_status"])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_prepare_rejects_stale_revision_and_corrupt_baseline(self):
        saved = self.save()
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "progress_revision_conflict")
        path = self.root / saved["path"]
        path.write_bytes(b"corrupt")
        with self.assertRaises(WorkError):
            self.prepare(revision=1)
        self.assertEqual(path.read_bytes(), b"corrupt")

    def test_prepare_rejects_reserved_history(self):
        history = self.root / "outputs/work/progress/example/task/history/1"
        history.mkdir(parents=True)
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "progress_save_pending")
        self.assertEqual(list(history.iterdir()), [])

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.value = fixtures.discussion()

    def save(self):
        preview = preview_progress(self.root, self.value, expected_revision=0)
        return save_progress(self.root, self.value, expected_revision=0,
                             approved_sha256=preview["approved_sha256"])

    def test_preview_is_read_only_and_binds_full_content(self):
        first = preview_progress(self.root, self.value, expected_revision=0)
        changed = copy.deepcopy(self.value)
        changed["open_questions"].append("Additional question")
        second = preview_progress(self.root, changed, expected_revision=0)
        self.assertNotEqual(first["approved_sha256"], second["approved_sha256"])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_read_rejects_history_mismatch(self):
        result = self.save()
        history = self.root / Path(result["path"]).parent / "history/1/progress.json"
        history.write_bytes(b"changed evidence")
        with self.assertRaises(WorkError) as caught:
            read_progress(self.root, "example", "task")
        self.assertEqual(caught.exception.code, "progress_history_mismatch")
        self.assertEqual(history.read_bytes(), b"changed evidence")

    def test_reserved_revision_blocks_preview_without_skipping_history(self):
        directory = self.root / "outputs/work/progress/example/task/history/1"
        directory.mkdir(parents=True)
        with self.assertRaises(WorkError) as caught:
            preview_progress(self.root, self.value, expected_revision=0)
        self.assertEqual(caught.exception.code, "progress_save_pending")
        self.assertEqual(caught.exception.details["committed_revision"], 0)
        self.assertEqual(list(directory.iterdir()), [])

    def test_read_error_has_progress_specific_code_and_cause(self):
        result = self.save()
        with patch.object(Path, "read_bytes", side_effect=OSError("unreadable")):
            with self.assertRaises(WorkError) as caught:
                read_progress(self.root, "example", "task")
        self.assertEqual(caught.exception.code, "progress_read_failed")
        self.assertIsInstance(caught.exception.__cause__, OSError)
        self.assertEqual(caught.exception.details["path"], str(self.root / result["path"]))
