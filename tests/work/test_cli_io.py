from __future__ import annotations

import argparse
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[2] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from cli_support import FileInputTestCase
from worklib.cli import build_parser, main
from worklib.contracts.attempt import canonicalize_attempt_contract
from worklib.contracts.correction import canonicalize_correction_contract
from worklib.foundation.errors import ExitCode
from worklib.foundation.markdown import render_json_contract


class CliFileTransportTests(FileInputTestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name) / "專案 workspace"
        self.root.mkdir()

    def invoke(self, *arguments, expected_code=0):
        stdout, stderr = io.StringIO(), io.StringIO()
        code = main(["--project-root", str(self.root), *arguments],
                    stdout=stdout, stderr=stderr)
        self.assertEqual(code, expected_code, stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        response = json.loads(stdout.getvalue())
        self.assertEqual(set(response), {"schema", "status", "reason_code", "message", "data"})
        self.assertEqual(response["schema"], "work-cli-result/v1")
        self.assertIsInstance(response["message"], str)
        self.assertIsInstance(response["data"], dict)
        return response

    def test_success_keeps_command_result_inside_data(self):
        result = self.invoke("paths", "resolve", "--requirement-id", "example")
        self.assertEqual(result["status"], "success")
        self.assertEqual(result["reason_code"], "ok")
        self.assertEqual(result["data"]["schema"], "work-paths/v1")
        self.assertEqual(result["data"]["requirement_id"], "example")

    def test_single_bom_and_no_bom_produce_identical_results(self):
        raw = b'{"decision":"general_only","selections":[]}'
        results = []
        for prefix in (b"", b"\xef\xbb\xbf"):
            results.append(self.invoke(
                "hierarchy", "selection-build", "--input-file", self.input_file(prefix + raw),
            ))
        self.assertEqual(results[0], results[1])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_invalid_encoding_and_json_are_rejected_without_artifact_writes(self):
        for raw, reason in (
            (b"\xff", "invalid_utf8"),
            ("{}".encode("utf-16"), "invalid_utf8"),
            (b"\xef\xbb\xbf\xef\xbb\xbf{}", "input_file_multiple_bom"),
            (b"", "invalid_json_contract"),
            (b"{", "invalid_json_contract"),
            (b'{"x":1,"x":2}', "duplicate_json_key"),
        ):
            with self.subTest(reason=reason, raw=raw):
                response = self.invoke(
                    "attempt", "render", "--input-file", self.input_file(raw),
                    expected_code=ExitCode.INPUT_FORMAT,
                )
                self.assertEqual(response["status"], "rejected")
                self.assertEqual(response["reason_code"], reason)
                self.assertEqual(list(self.root.iterdir()), [])

    def test_non_regular_or_missing_file_never_dispatches_or_acquires_lock(self):
        directory = self.root / "execution"
        directory.mkdir()
        for path in (self.root / "missing.json", directory):
            with self.subTest(path=path), patch("worklib.cli.run_execute") as operation:
                response = self.invoke(
                    "execute", "record-finish", "--input-file", str(path),
                    "--user-config-root", str(self.root),
                    "--task-path", "outputs/work/tasks/example/task.json",
                    "--execution-dir", "execution", "--task-id", "TASK-001",
                    expected_code=ExitCode.IO_FAILURE,
                )
                self.assertEqual(response["status"], "failed")
                self.assertEqual(response["reason_code"], "input_file_read_failed")
                operation.assert_not_called()
                self.assertEqual(list(directory.iterdir()), [])

    def test_unreadable_file_retains_io_error_category(self):
        path = self.input_file("{}")
        with patch("worklib.foundation.cli_io.Path.read_bytes", side_effect=PermissionError):
            response = self.invoke(
                "attempt", "render", "--input-file", path,
                expected_code=ExitCode.IO_FAILURE,
            )
        self.assertEqual(response["reason_code"], "input_file_read_failed")

    def test_removed_stdin_returns_migration_hint_without_reading(self):
        for option in ("--stdin", "--stdin=true"):
            with self.subTest(option=option), patch("sys.stdin") as legacy_input:
                response = self.invoke(
                    "task", "draft-init", option, expected_code=ExitCode.CLI_USAGE,
                )
                self.assertEqual(response["reason_code"], "stdin_removed")
                self.assertEqual(response["data"]["replacement"], "--input-file")
                legacy_input.read.assert_not_called()

    def test_missing_input_and_conflicting_sources_are_usage_errors(self):
        for arguments in (
            ("attempt", "render"),
            ("attempt", "render", "--input-file"),
            ("attempt", "validate", "--path", "formal.json", "--input-file", "request.json"),
        ):
            with self.subTest(arguments=arguments):
                response = self.invoke(*arguments, expected_code=ExitCode.CLI_USAGE)
                self.assertEqual(response["reason_code"], "cli_usage_error")

    def test_unexpected_exception_is_structured_without_exception_contents(self):
        with patch("worklib.cli._run", side_effect=RuntimeError("private exception contents")):
            response = self.invoke(
                "paths", "resolve", "--requirement-id", "example",
                expected_code=ExitCode.INTERNAL_ERROR,
            )
        self.assertEqual(response["status"], "failed")
        self.assertEqual(response["reason_code"], "internal_error")
        self.assertNotIn("private exception contents", json.dumps(response))

    def test_existing_already_completed_result_keeps_zero_exit(self):
        with patch("worklib.cli._run", return_value={"status": "already_completed", "record_id": "R-1"}):
            response = self.invoke("paths", "resolve", "--requirement-id", "example")
        self.assertEqual(response["status"], "already_completed")
        self.assertEqual(response["reason_code"], "already_completed")
        self.assertEqual(response["data"]["record_id"], "R-1")

    def test_every_command_help_is_one_json_response_without_required_inputs(self):
        def commands(parser, prefix=()):
            yield prefix
            for action in parser._actions:
                if isinstance(action, argparse._SubParsersAction):
                    for name, child in action.choices.items():
                        yield from commands(child, (*prefix, name))

        for command in commands(build_parser()):
            with self.subTest(command=command):
                stdout, stderr = io.StringIO(), io.StringIO()
                code = main([*command, "--help"], stdout=stdout, stderr=stderr)
                self.assertEqual(code, 0)
                self.assertEqual(stderr.getvalue(), "")
                response = json.loads(stdout.getvalue())
                self.assertEqual(response["schema"], "work-cli-result/v1")
                self.assertEqual(response["status"], "success")
                self.assertIn("usage:", response["data"]["help"])

    def test_real_process_uses_cwd_request_paths_and_utf8_without_shell(self):
        raw = json.dumps({"decision": "general_only", "selections": []})
        path = Path(self.input_file(raw))
        command = [
            sys.executable, "-B", str(SCRIPT_ROOT / "work.py"), "--project-root", str(self.root),
            "hierarchy", "selection-build", "--input-file", path.name,
        ]
        for failure in (False, True):
            with self.subTest(failure=failure):
                arguments = command if not failure else command[:-1] + ["找不到 missing.json"]
                result = subprocess.run(
                    arguments, cwd=path.parent, stdin=subprocess.DEVNULL,
                    capture_output=True, shell=False, timeout=30,
                    env={**os.environ, "PYTHONIOENCODING": "ascii", "PYTHONUTF8": "0"},
                )
                self.assertEqual(result.returncode, ExitCode.IO_FAILURE if failure else 0)
                self.assertEqual(result.stderr, b"")
                self.assertFalse(result.stdout.startswith(b"\xef\xbb\xbf"))
                self.assertNotIn(b"\r\n", result.stdout)
                response = json.loads(result.stdout.decode("utf-8"))
                self.assertEqual(response["schema"], "work-cli-result/v1")
                if failure:
                    self.assertIn("找不到".encode("utf-8"), result.stdout)
                    self.assertEqual(response["reason_code"], "input_file_read_failed")

    def test_renderer_data_preserves_canonical_order_and_round_trips(self):
        attempt = {
            "schema": "work-attempt/v1", "attempt_id": "ATTEMPT-001",
            "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001", "skill_id": None,
            "status": "in_progress", "task_sha256": "a" * 64,
            "task_instructions_sha256": "b" * 64, "execute_instructions_sha256": "c" * 64,
            "hierarchy_selection_sha256": "f" * 64, "execute_skill_selection_sha256": "d" * 64,
            "started_at": "2026-09-01T10:00+08:00", "records": [],
        }
        correction = {
            "schema": "work-correction/v1", "correction_id": "ATTEMPT-001-CORRECTION-001",
            "created_at": "2026-09-01T10:05+08:00", "target_attempt_id": "ATTEMPT-001",
            "task_instructions_sha256": "b" * 64, "execute_instructions_sha256": "c" * 64,
            "field": "records[0].outcome", "correct_value": "passed", "reason": "修正紀錄。",
        }
        handoff = {
            "schema": "work-handoff/v1", "marker": "WORK-HANDOFF", "direction": "plan_to_task",
            "requirement_id": "example",
            "artifacts": {"plan": "outputs/work/plans/example.json",
                          "task": "outputs/work/tasks/example/task.json",
                          "execution": "outputs/work/executions/example"},
            "source": {"stage": "plan", "plan_sha256": "a" * 64, "skill_selection_sha256": "d" * 64},
            "target": {"stage": "task"}, "summary": "繼續討論。", "affected_ids": ["GOAL-001"],
        }
        cases = (
            ("attempt", attempt, canonicalize_attempt_contract(attempt, project_root=self.root)),
            ("correction", correction, canonicalize_correction_contract(correction)),
            ("handoff", handoff, json.loads(json.dumps(handoff, sort_keys=True))),
        )
        for command, contract, canonical in cases:
            with self.subTest(command=command):
                response = self.invoke(
                    command, "render", "--input-file", self.input_file(json.dumps(contract)),
                )
                raw = render_json_contract(response["data"])
                self.assertEqual(raw, render_json_contract(canonical))
                self.assertNotIn(b"work-cli-result/v1", raw)
                self.invoke(command, "validate", "--input-file", self.input_file(raw))


if __name__ == "__main__":
    unittest.main()
