from __future__ import annotations

import copy
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.cli import main
from worklib.contracts.progress import validate_progress_contract
from worklib.foundation.errors import ExitCode
from worklib.foundation.markdown import render_json_contract
from worklib.foundation.spec_update import state_writer


class ProgressCliTests(FileInputTestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.directory = self.root / "outputs/work/progress/example/plan"
        self.progress = {
            "schema": "work-discussion-progress/v1", "requirement_id": "example",
            "mode": "plan", "revision": 1, "status": "discussion_only",
            "title": "保存尚未完成的討論", "request": "先記錄目前共識，稍後繼續。",
            "current_task_id": None, "context": {"scope": ["需求規劃"]},
            "source_status": ["Migration is pending; acceptance decisions are missing."],
            "notes": ["具體討論細節。"], "confirmed_decisions": [{"statement": "保留已確認需求。"}],
            "tentative": ["候選驗收方式尚未決定。"], "open_questions": ["哪些結果可供觀察？"],
            "next_discussion_point": "繼續確認驗收結果。",
        }

    def cli(self, *arguments, payload=None, expected_code=0, raw=None):
        output, errors = io.StringIO(), io.StringIO()
        code = main(
            self.input_arguments(["--project-root", str(self.root), *arguments], raw if raw is not None else json.dumps(payload, ensure_ascii=False)),
            stdout=output, stderr=errors,
        )
        self.assertEqual(code, expected_code, errors.getvalue())
        self.assertEqual(errors.getvalue(), "")
        response = json.loads(output.getvalue())
        self.assertEqual(response["schema"], "work-cli-result/v1")
        return response if expected_code else response["data"]

    def preview(self, progress=None, expected_revision=0, expected_code=0):
        return self.cli("progress", "validate", "--input-file", "request.json", "--expected-revision", str(expected_revision),
                        payload=progress or self.progress, expected_code=expected_code)

    def save(self, progress=None, expected_revision=0, approval=None, expected_code=0):
        progress = progress or self.progress
        if approval is None:
            approval = self.preview(progress, expected_revision)["approved_sha256"]
        return self.cli("progress", "save", "--input-file", "request.json", "--expected-revision", str(expected_revision),
                        "--approved-sha256", approval, payload=progress, expected_code=expected_code)

    def read(self, mode="plan", requirement_id="example", expected_code=0):
        return self.cli("progress", "read", "--requirement-id", requirement_id,
                        "--mode", mode, expected_code=expected_code)

    def snapshot(self):
        return {path.relative_to(self.root).as_posix(): path.read_bytes()
                for path in self.root.rglob("*") if path.is_file()}

    def test_preview_unfinished_plan_is_read_only_without_formal_artifacts(self):
        preview = self.preview()
        self.assertEqual(preview["progress"], self.progress)
        self.assertEqual(preview["status"], "valid")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_save_and_fresh_process_resume_preserve_decision_status_and_details(self):
        result = self.save()
        command = [sys.executable, "-B", str(SCRIPT_ROOT / "work.py"), "--project-root", str(self.root),
                   "progress", "read", "--requirement-id", "example", "--mode", "plan"]
        restored = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", timeout=30,
                                  env={**os.environ, "PYTHONIOENCODING": "utf-8"})
        self.assertEqual(restored.returncode, 0, restored.stderr)
        content = json.loads(restored.stdout)["data"]
        self.assertEqual(content["progress"], self.progress)
        self.assertEqual(content["sha256"], result["sha256"])
        self.assertEqual(set(path.name for path in (self.root / "outputs/work").iterdir()), {"progress"})

    def test_plan_task_and_requirement_histories_are_independent(self):
        plan = self.save()
        task = copy.deepcopy(self.progress)
        task.update(mode="task", current_task_id="TASK-001", title="Task discussion")
        self.save(task)
        other = copy.deepcopy(self.progress)
        other.update(requirement_id="other", title="Other requirement")
        self.save(other)
        self.assertEqual(self.read()["sha256"], plan["sha256"])
        self.assertEqual(self.read("task")["progress"], task)
        self.assertEqual(self.read(requirement_id="other")["progress"], other)

    def test_second_save_preserves_history_and_full_continuation(self):
        first = self.save()
        old = (self.directory / "progress.json").read_bytes()
        updated = copy.deepcopy(self.progress)
        updated["revision"] = 2
        updated["confirmed_decisions"].append({"statement": "New decision", "rationale": "User supplied reason"})
        updated["next_discussion_point"] = "Discuss remaining constraints."
        self.save(updated, expected_revision=1)
        self.assertEqual((self.directory / "history/1/progress.json").read_bytes(), old)
        self.assertEqual(self.read()["progress"], updated)
        self.assertNotEqual(self.read()["sha256"], first["sha256"])

    def test_unavailable_formal_sources_and_active_execution_state_do_not_block_memory(self):
        # These files deliberately cannot pass formal validation. Progress must
        # neither require their validity nor touch any of them.
        for relative, raw in {
            "outputs/work/plans/example.json": b'{"old_instruction_sources":true}\n',
            "outputs/work/tasks/example/task.json": b'{"migration":"incomplete"}\n',
            "outputs/work/tasks/example/drafts/index.json": b'{"source":"stale"}\n',
            "outputs/work/executions/example/index.json": b'{"lock":{"kind":"spec_update"}}\n',
            "outputs/work/executions/example/.work-spec-update-SPEC-UPDATE-001.json": b"unfinished transaction",
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json": b"active historical attempt",
        }.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        before = self.snapshot()
        self.progress.update(mode="task", current_task_id="TASK-001")
        self.progress["context"] = {"source_plan": {"canonical_sha256": "0" * 64}, "skill_selection": {"unavailable": True}}
        self.save()
        self.assertEqual(self.read("task")["progress"], self.progress)
        after = self.snapshot()
        self.assertEqual({path: after[path] for path in before}, before)

    def test_progress_is_rejected_by_formal_plan_and_task_validators(self):
        for mode, path_option, path_value in (
            ("plan", "--plan-path", "outputs/work/plans/example.json"),
            ("task", "--task-path", "outputs/work/tasks/example/task.json"),
        ):
            with self.subTest(mode=mode):
                self.cli(mode, "validate", "--input-file", "request.json", path_option, path_value,
                         "--user-config-root", str(self.root), payload=self.progress,
                         expected_code=ExitCode.CONTRACT)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_changed_content_invalidates_approval_before_any_write(self):
        approval = self.preview()["approved_sha256"]
        self.progress["tentative"].append("Unreviewed candidate")
        error = self.save(approval=approval, expected_code=ExitCode.ARTIFACT_INTEGRITY)
        self.assertEqual(error["reason_code"], "progress_approval_changed")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_new_saved_revision_rejects_stale_writer_without_overwrite(self):
        approval = self.preview()["approved_sha256"]
        self.save(approval=approval)
        before = self.snapshot()
        error = self.save(approval=approval, expected_code=ExitCode.WORKFLOW_STATE)
        self.assertEqual(error["reason_code"], "progress_revision_conflict")
        self.assertEqual(self.snapshot(), before)

    def test_baseline_content_is_bound_to_approval(self):
        self.save()
        candidate = copy.deepcopy(self.progress)
        candidate["revision"] = 2
        approval = self.preview(candidate, 1)["approved_sha256"]
        changed = copy.deepcopy(self.progress)
        changed["notes"].append("Changed outside the reviewed baseline")
        raw = render_json_contract(validate_progress_contract(changed))
        for path in (self.directory / "progress.json", self.directory / "history/1/progress.json"):
            path.write_bytes(raw)
        before = self.snapshot()
        error = self.save(candidate, 1, approval, ExitCode.ARTIFACT_INTEGRITY)
        self.assertEqual(error["reason_code"], "progress_approval_changed")
        self.assertEqual(self.snapshot(), before)

    def test_corrupt_current_file_is_not_overwritten(self):
        self.save()
        current = self.directory / "progress.json"
        current.write_bytes(b"incomplete unrelated edit")
        before = self.snapshot()
        self.read(expected_code=ExitCode.INPUT_FORMAT)
        self.progress["revision"] = 2
        self.preview(expected_revision=1, expected_code=ExitCode.INPUT_FORMAT)
        self.assertEqual(self.snapshot(), before)

    def test_history_mismatch_blocks_read_and_save(self):
        self.save()
        (self.directory / "history/1/progress.json").write_bytes(b"changed history")
        error = self.read(expected_code=ExitCode.ARTIFACT_INTEGRITY)
        self.assertEqual(error["reason_code"], "progress_history_mismatch")
        self.progress["revision"] = 2
        self.preview(expected_revision=1, expected_code=ExitCode.ARTIFACT_INTEGRITY)

    def test_interrupted_replacement_keeps_previous_commit_and_blocks_retry(self):
        self.save()
        previous = self.read()
        self.progress["revision"] = 2
        approval = self.preview(expected_revision=1)["approved_sha256"]
        with patch("worklib.artifacts.progress.os.replace", side_effect=OSError("interrupted")):
            error = self.save(expected_revision=1, approval=approval, expected_code=ExitCode.IO_FAILURE)
        self.assertEqual(error["reason_code"], "progress_save_interrupted")
        self.assertEqual(self.read(), previous)
        before = self.snapshot()
        rejected = self.save(expected_revision=1, approval=approval, expected_code=ExitCode.WORKFLOW_STATE)
        self.assertEqual(rejected["reason_code"], "progress_save_pending")
        self.assertEqual(self.snapshot(), before)

    def test_partial_first_save_never_looks_like_committed_progress(self):
        approval = self.preview()["approved_sha256"]

        def short_write(path, raw):
            path.write_bytes(raw[:10])
            raise OSError("disk full")

        with patch("worklib.artifacts.progress._write", side_effect=short_write):
            self.save(approval=approval, expected_code=ExitCode.IO_FAILURE)
        self.assertEqual(self.read(expected_code=ExitCode.WORKFLOW_STATE)["reason_code"], "progress_not_saved")
        self.assertEqual(self.preview(expected_code=ExitCode.WORKFLOW_STATE)["reason_code"], "progress_save_pending")

    def test_writer_mutex_prevents_concurrent_process_publication(self):
        approval = self.preview()["approved_sha256"]
        self.directory.mkdir(parents=True)
        command = [sys.executable, "-B", str(SCRIPT_ROOT / "work.py"), "--project-root", str(self.root),
                   "progress", "save", "--input-file", "request.json", "--expected-revision", "0", "--approved-sha256", approval]
        with state_writer(self.root, self.directory.relative_to(self.root).as_posix()):
            result = subprocess.run(self.input_arguments(command, json.dumps(self.progress)),
                                    capture_output=True, text=True, encoding="utf-8",
                                    shell=False, timeout=30)
        self.assertEqual(result.returncode, ExitCode.LOCK_CONFLICT, result.stderr)
        self.assertEqual(json.loads(result.stdout)["reason_code"], "work_state_writer_busy")
        self.assertFalse((self.directory / "progress.json").exists())
        self.assertFalse((self.directory / "history").exists())
        self.save(approval=approval)

    def test_missing_read_is_read_only(self):
        error = self.read(expected_code=ExitCode.WORKFLOW_STATE)
        self.assertEqual(error["reason_code"], "progress_not_saved")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_invalid_contracts_do_not_create_files(self):
        for field, value in (
            ("schema", "work-plan/v1"), ("status", "confirmed"), ("mode", "execute"),
            ("revision", True), ("revision", 0), ("current_task_id", "TASK-001"),
            ("notes", "not an array"), ("context", []), ("next_discussion_point", " "),
            ("confirmed_decisions", [{"statement": "Known", "approved": True}]),
            ("confirmed_decisions", [{"statement": "Known", "rationale": " "}]),
            ("requirement_id", "../outside"), ("requirement_id", "CON"),
        ):
            with self.subTest(field=field, value=value):
                candidate = copy.deepcopy(self.progress)
                candidate[field] = value
                self.preview(candidate, expected_code=ExitCode.CONTRACT)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_negative_expected_revision_and_revision_gaps_are_rejected(self):
        self.preview(expected_revision=-1, expected_code=ExitCode.CONTRACT)
        self.progress["revision"] = 2
        error = self.preview(expected_code=ExitCode.WORKFLOW_STATE)
        self.assertEqual(error["reason_code"], "progress_revision_conflict")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_duplicate_keys_and_missing_cli_arguments_are_rejected(self):
        self.cli("progress", "validate", "--input-file", "request.json", "--expected-revision", "0",
                 raw='{"mode":"plan","mode":"task"}', expected_code=ExitCode.INPUT_FORMAT)
        for arguments in (("save", "--input-file", "request.json", "--expected-revision", "0"),
                          ("validate", "--input-file", "request.json"), ("read", "--requirement-id", "example"),
                          ("read", "--requirement-id", "example", "--mode", "execute")):
            with self.subTest(arguments=arguments):
                self.cli("progress", *arguments, expected_code=ExitCode.CLI_USAGE)

    def test_storage_link_cannot_alias_another_requirement(self):
        target = self.root / "other-storage"
        target.mkdir()
        self.directory.parent.mkdir(parents=True)
        try:
            self.directory.symlink_to(target, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"Directory symlinks unavailable: {error}")
        self.preview(expected_code=ExitCode.CONTRACT)
        self.assertEqual(list(target.iterdir()), [])

    def test_current_file_hard_link_is_rejected(self):
        self.save()
        current = self.directory / "progress.json"
        try:
            os.link(current, self.root / "alias.json")
        except OSError as error:
            self.skipTest(f"Hard links unavailable: {error}")
        self.read(expected_code=ExitCode.CONTRACT)


if __name__ == "__main__":
    unittest.main()
