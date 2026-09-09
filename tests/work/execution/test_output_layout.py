from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"
sys.path.insert(0, str(SKILL_ROOT / "scripts"))

from worklib.contracts.attempt import validate_attempt_file
from worklib.contracts.correction import validate_correction_file
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
    validate_execution_index,
)
from worklib.execution.attempt_close import close_attempt
from worklib.execution.attempt_start import recover_attempt_start, start_attempt
from worklib.execution.correction import create_correction
from worklib.execution.record_begin import begin_record
from worklib.execution.record_finish import finish_record
from worklib.execution.recovery import recover_execution
from worklib.foundation.errors import ExitCode, WorkError
from worklib.foundation.markdown import render_json_contract
from worklib.instructions.selection import build_instruction_selection


class ExecutionOutputLayoutTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.project = Path(temporary.name).resolve()
        self.execution = self.project / "execution"
        self.execution.mkdir()
        self.task_directory = self.execution / "TASK-001"
        self.attempt_path = self.task_directory / "ATTEMPT-001" / "attempt.json"
        self.index_path = self.execution / "index.json"
        task_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="task",
            selected_paths=[],
            reference_names=["task.general.task-records"],
        )
        execute_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="execute",
            selected_paths=[],
            reference_names=["execute.general.execution-records"],
        )
        self.contract = {
            "requirement_id": "example",
            "spec_id": "TASK-SPEC-001",
            "artifacts": {"task": "task.json", "execution": "execution"},
            "tasks": [{
                "id": "TASK-001",
                "skill_id": None,
                "instruction_selection": task_selection,
                "validations": [{"id": "VAL-001"}],
            }],
        }
        self.validation = {
            "task_sha256": "a" * 64,
            "instructions_sha256": "b" * 64,
            "task_instructions_sha256": {
                "TASK-001": task_selection["instructions_sha256"],
            },
            "hierarchy_selection_sha256": "f" * 64,
            "skill_selection_sha256": "d" * 64,
            "task_skill_ids": {"TASK-001": None},
        }
        self.preflight = {
            "task_spec_id": "TASK-SPEC-001",
            "task_id": "TASK-001",
            "skill_id": None,
            "task_sha256": self.validation["task_sha256"],
            "task_instructions_sha256": task_selection["instructions_sha256"],
            "execute_instructions_sha256": execute_selection["instructions_sha256"],
            "hierarchy_selection_sha256": "f" * 64,
            "execute_skill_selection": {"selection_sha256": "d" * 64},
            "execution_dir": "execution",
            "snapshot_sha256": "e" * 64,
        }
        self.request = {
            "schema": "work-attempt-start-request/v1",
            "worktree_snapshot_sha256": self.preflight["snapshot_sha256"],
        }
        self.common = {
            "project_root": self.project,
            "user_config_root": temporary.name,
            "raw_task_path": "task.json",
            "raw_execution_dir": "execution",
            "task_id": "TASK-001",
        }
        (self.project / "task.json").write_bytes(render_json_contract(self.contract))
        index = build_initial_execution_index(self.contract, self.validation)
        self.index_path.write_bytes(render_execution_index(index))
        # Isolate upstream Plan/TASK and Git review. All artifact reads, writes,
        # canonical validation, instruction checks, locks, and recovery are real.
        for module in ("record_begin", "record_finish", "attempt_close", "correction"):
            mocked = patch(
                f"worklib.execution.{module}.validate_task_contract",
                return_value=self.validation,
            )
            mocked.start()
            self.addCleanup(mocked.stop)

    def start(self, request=None):
        with patch(
            "worklib.execution.attempt_start.inspect_execute_worktree",
            return_value=self.preflight,
        ):
            return start_attempt(
                json.dumps(request or self.request).encode("utf-8"),
                source="test",
                **self.common,
            )

    def finish_validation(self) -> None:
        reserved = begin_record(base_record_id="VAL-001", **self.common)
        result = finish_record(
            json.dumps({
                "schema": "work-record-finish-request/v1",
                "record": {
                    "id": reserved["record_id"],
                    "kind": "validation",
                    "outcome": "passed",
                    "evidence": "The approved check passed.",
                },
            }).encode("utf-8"),
            source="test",
            **self.common,
        )
        self.assertEqual(result["lock_status"], "attempt_held")

    def close(self, *, stopped=False):
        request = {"schema": "work-attempt-close-request/v1", "status": "completed"}
        if stopped:
            request.update({
                "status": "stopped",
                "final_type": "user_stopped",
                "reason": "The user paused execution.",
            })
        return close_attempt(
            json.dumps(request).encode("utf-8"), source="test", **self.common
        )

    def correct(self, attempt_id="ATTEMPT-001"):
        return create_correction(
            json.dumps({
                "schema": "work-correction-create-request/v1",
                "target_attempt_id": attempt_id,
                "field": "records[0].evidence",
                "correct_value": "The approved manual check passed.",
                "reason": "Clarify the recorded evidence.",
                "invalidates_completion": False,
            }).encode("utf-8"),
            source="test",
            **self.common,
        )

    def read_index(self):
        raw = self.index_path.read_bytes()
        validate_execution_index(raw, source=str(self.index_path))
        return json.loads(raw)

    def recover_start(self):
        with patch(
            "worklib.execution.attempt_start.execute_preflight",
            return_value=self.preflight,
        ), patch("worklib.execution.attempt_start._validate_snapshot"):
            return recover_attempt_start(
                json.dumps(self.request).encode("utf-8"),
                source="test",
                **self.common,
            )

    def test_lifecycle_groups_corrections_and_preserves_closed_attempt(self) -> None:
        result = self.start()
        self.assertEqual(
            result["attempt_path"], "execution/TASK-001/ATTEMPT-001/attempt.json"
        )
        self.assertFalse((self.attempt_path.parent / "corrections").exists())
        self.finish_validation()
        self.close()
        original = self.attempt_path.read_bytes()
        for number in (1, 2):
            result = self.correct()
            expected = (
                "execution/TASK-001/ATTEMPT-001/corrections/"
                f"ATTEMPT-001-CORRECTION-{number:03d}.json"
            )
            self.assertEqual(result["correction_path"], expected)
            self.assertEqual(
                validate_correction_file(self.project, expected)["result"], "valid"
            )
            self.assertEqual(self.attempt_path.read_bytes(), original)
        self.assertEqual(self.read_index()["overall_status"], "completed")
        self.assertNotIn("lock", self.read_index())
        self.assertEqual(list(self.execution.glob(".work-*.tmp")), [])

    def test_retry_uses_next_directory_and_keeps_source(self) -> None:
        self.start()
        self.finish_validation()
        self.close(stopped=True)
        original = self.attempt_path.read_bytes()
        request = copy.deepcopy(self.request)
        request["continuation"] = {
            "source_attempt_id": "ATTEMPT-001",
            "carried_records": [{
                "record_id": "VAL-001",
                "evidence": "The previous check remains valid.",
            }],
        }
        recovery_selection = build_instruction_selection(
            skill_root=SKILL_ROOT,
            mode="execute",
            selected_paths=[],
            reference_names=[
                "execute.general.execution-records",
                "execute.general.execution-recovery",
            ],
        )
        self.preflight["execute_instructions_sha256"] = recovery_selection["instructions_sha256"]
        result = self.start(request)
        self.assertEqual(
            result["attempt_path"], "execution/TASK-001/ATTEMPT-002/attempt.json"
        )
        attempt = json.loads((self.project / result["attempt_path"]).read_bytes())
        self.assertEqual(attempt["continued_from"], "ATTEMPT-001")
        self.assertEqual(attempt["carried_records"][0]["record_id"], "VAL-001")
        self.close()
        correction = self.correct("ATTEMPT-002")
        self.assertEqual(correction["correction_id"], "ATTEMPT-002-CORRECTION-001")
        self.assertEqual(self.attempt_path.read_bytes(), original)

    def test_start_recovers_after_attempt_directory_creation_failure(self) -> None:
        real_mkdir = Path.mkdir

        def interrupted_mkdir(path, *args, **kwargs):
            if path.name == "ATTEMPT-001":
                raise OSError("simulated directory failure")
            return real_mkdir(path, *args, **kwargs)

        with patch.object(Path, "mkdir", interrupted_mkdir):
            with self.assertRaises(WorkError) as context:
                self.start()
        self.assertTrue(context.exception.details["recovery_required"])
        self.assertEqual(context.exception.code, "attempt_start_attempt_directory_failed")
        self.assertIn("lock", self.read_index())
        self.assertFalse(self.attempt_path.exists())
        result = self.recover_start()
        self.assertEqual(result["status"], "recovered")
        self.assertTrue(self.attempt_path.is_file())
        self.assertEqual(self.read_index()["tasks"][0]["status"], "in_progress")

    def test_start_recovers_with_empty_attempt_directory(self) -> None:
        with patch(
            "worklib.execution.attempt_start._write_exclusive",
            side_effect=WorkError(ExitCode.IO_FAILURE, "interrupted", "Simulated failure."),
        ):
            with self.assertRaises(WorkError) as context:
                self.start()
        self.assertTrue(context.exception.details["recovery_required"])
        self.assertTrue(self.attempt_path.parent.is_dir())
        self.assertFalse(self.attempt_path.exists())
        self.assertEqual(self.recover_start()["status"], "recovered")
        validate_attempt_file(
            self.project, "execution/TASK-001/ATTEMPT-001/attempt.json"
        )

    def test_start_rejects_existing_empty_attempt_directory_without_writing(self) -> None:
        self.attempt_path.parent.mkdir(parents=True)
        original = self.index_path.read_bytes()
        with self.assertRaises(WorkError) as context:
            self.start()
        self.assertEqual(context.exception.code, "attempt_start_unexpected_attempt_history")
        self.assertEqual(self.index_path.read_bytes(), original)
        self.assertFalse(self.attempt_path.exists())

    def test_start_rejects_flat_history_without_writing(self) -> None:
        self.task_directory.mkdir()
        legacy = self.task_directory / "ATTEMPT-001.json"
        legacy.write_bytes(b"legacy")
        original = self.index_path.read_bytes()
        with self.assertRaises(WorkError) as context:
            self.start()
        self.assertEqual(context.exception.code, "execution_legacy_layout_unsupported")
        self.assertEqual(legacy.read_bytes(), b"legacy")
        self.assertEqual(self.index_path.read_bytes(), original)
        self.assertFalse(self.attempt_path.parent.exists())

    def test_record_begin_rejects_mixed_legacy_corrections_without_writing(self) -> None:
        self.start()
        legacy = self.task_directory / "ATTEMPT-001-CORRECTION-001.json"
        legacy.write_bytes(b"legacy")
        original_index = self.index_path.read_bytes()
        original_attempt = self.attempt_path.read_bytes()
        with self.assertRaises(WorkError) as context:
            begin_record(base_record_id="VAL-001", **self.common)
        self.assertEqual(context.exception.code, "execution_legacy_layout_unsupported")
        self.assertEqual(self.index_path.read_bytes(), original_index)
        self.assertEqual(self.attempt_path.read_bytes(), original_attempt)
        self.assertEqual(legacy.read_bytes(), b"legacy")

    def test_attempt_validator_rejects_legacy_filename(self) -> None:
        self.start()
        legacy = self.task_directory / "ATTEMPT-001.json"
        legacy.write_bytes(self.attempt_path.read_bytes())
        with self.assertRaises(WorkError) as context:
            validate_attempt_file(
                self.project, "execution/TASK-001/ATTEMPT-001.json"
            )
        self.assertEqual(context.exception.code, "attempt_filename_mismatch")

    def test_attempt_validator_checks_attempt_and_task_directory_identity(self) -> None:
        self.start()
        for relative, error in (
            ("execution/TASK-001/ATTEMPT-002/attempt.json", "attempt_parent_attempt_mismatch"),
            ("execution/TASK-002/ATTEMPT-001/attempt.json", "attempt_parent_task_mismatch"),
        ):
            with self.subTest(path=relative):
                path = self.project / relative
                path.parent.mkdir(parents=True)
                path.write_bytes(self.attempt_path.read_bytes())
                with self.assertRaises(WorkError) as context:
                    validate_attempt_file(self.project, relative)
                self.assertEqual(context.exception.code, error)

    def test_correction_validator_rejects_flat_and_wrong_attempt_directories(self) -> None:
        self.start()
        self.finish_validation()
        self.close()
        created = self.correct()
        raw = (self.project / created["correction_path"]).read_bytes()
        for relative in (
            "execution/TASK-001/ATTEMPT-001-CORRECTION-001.json",
            "execution/TASK-001/ATTEMPT-002/corrections/ATTEMPT-001-CORRECTION-001.json",
        ):
            with self.subTest(path=relative):
                path = self.project / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(raw)
                with self.assertRaises(WorkError) as context:
                    validate_correction_file(self.project, relative)
                self.assertEqual(context.exception.code, "correction_parent_attempt_mismatch")

    def test_correction_recovers_after_directory_and_install_failures(self) -> None:
        self.start()
        self.finish_validation()
        self.close()
        original = self.attempt_path.read_bytes()
        real_mkdir = Path.mkdir

        def interrupted_mkdir(path, *args, **kwargs):
            if path.name == "corrections":
                raise OSError("simulated directory failure")
            return real_mkdir(path, *args, **kwargs)

        failures = (
            patch.object(Path, "mkdir", interrupted_mkdir),
            patch("os.link", side_effect=OSError("simulated install failure")),
        )
        for number, failure in enumerate(failures, start=1):
            with self.subTest(number=number):
                with failure:
                    with self.assertRaises(WorkError) as context:
                        self.correct()
                self.assertTrue(context.exception.details["recovery_required"])
                self.assertEqual(self.read_index()["lock"]["kind"], "correction")
                result = recover_execution(
                    json.dumps({
                        "schema": "work-execution-recovery-request/v1",
                        "transaction": "correction",
                        "attempt_id": "ATTEMPT-001",
                        "transaction_files": sorted(
                            path.name for path in self.execution.glob(".work-*.tmp")
                        ),
                    }).encode("utf-8"),
                    source="test",
                    **self.common,
                )
                self.assertEqual(result["status"], "recovered")
                self.assertEqual(
                    result["correction_id"], f"ATTEMPT-001-CORRECTION-{number:03d}"
                )
                validate_correction_file(self.project, result["correction_path"])
                self.assertEqual(self.attempt_path.read_bytes(), original)
                self.assertNotIn("lock", self.read_index())
                self.assertEqual(list(self.execution.glob(".work-*.tmp")), [])


if __name__ == "__main__":
    unittest.main()
