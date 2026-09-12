from __future__ import annotations

import copy
import hashlib
import io
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills" / "work" / "scripts"))

from contracts import test_task as task_fixture
from worklib.cli import main
from worklib.contracts.execution_index import build_initial_execution_index, render_execution_index
from worklib.contracts.task import render_task_contract, validate_task_contract
from worklib.contracts.task_diagnostics import diagnose_task_file
from worklib.foundation.errors import WorkError
from worklib.execution.worktree import inspect_execute_worktree


class TaskDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        fixture = task_fixture.TaskInstructionContractTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.root = fixture.project_root
        self.artifacts = fixture.artifacts
        self.contract = fixture.contract
        self.task_path = self.root / self.artifacts["task"]
        self.task_path.parent.mkdir(parents=True)
        self.raw = render_task_contract(self.contract)
        self.task_path.write_bytes(self.raw)
        self.index = build_initial_execution_index(self.contract, fixture.validate())
        self.directory = self.root / self.artifacts["execution"]
        self.directory.mkdir(parents=True)
        self.index_path = self.directory / "index.json"
        self.save_index()

    def save_index(self):
        self.index_path.write_bytes(render_execution_index(self.index))

    def snapshot(self):
        return {
            path.relative_to(self.root).as_posix(): path.read_bytes() if path.is_file() else None
            for path in self.root.rglob("*")
        }

    def diagnose(self):
        before = self.snapshot()
        result = diagnose_task_file(
            self.root, str(self.root), self.artifacts["task"],
            plan_path=self.artifacts["plan"], execution_dir=self.artifacts["execution"],
        )
        self.assertEqual(self.snapshot(), before)
        json.dumps(result, allow_nan=False)
        return result

    def cli(self, arguments):
        output, errors = io.StringIO(), io.StringIO()
        code = main(["--project-root", str(self.root), *arguments], stdout=output, stderr=errors)
        self.assertEqual(errors.getvalue(), "")
        return code, json.loads(output.getvalue())

    def test_valid_report_and_cli_are_read_only(self):
        result = self.diagnose()
        self.assertTrue(result["normal_use_allowed"])
        self.assertEqual(result["contract_status"], "passed")
        self.assertEqual(result["repair_mode"], "review_required")
        before = self.snapshot()
        code, envelope = self.cli([
            "task", "diagnose", "--path", self.artifacts["task"],
            "--plan-path", self.artifacts["plan"], "--execution-dir", self.artifacts["execution"],
            "--user-config-root", str(self.root),
        ])
        self.assertEqual(code, 0)
        self.assertEqual(envelope["data"], result)
        self.assertEqual(self.snapshot(), before)

    def test_independent_missing_fields_and_legacy_fields_are_aggregated(self):
        damaged = copy.deepcopy(self.contract)
        del damaged["summary"]
        del damaged["tasks"][0]["goal"]
        damaged["tasks"][0]["validations"] = []
        damaged["rule_selection"] = {"preserve": "original"}
        self.task_path.write_bytes(json.dumps(damaged).encode("utf-8") + b"\n")
        result = self.diagnose()
        locations = {issue["location"] for issue in result["issues"]}
        self.assertTrue({"/summary", "/tasks/0/goal", "/tasks/0/validations", "/rule_selection"} <= locations)
        self.assertTrue(any(issue["code"] == "legacy_field" for issue in result["issues"]))
        self.assertFalse(result["normal_use_allowed"])
        self.assertEqual(result["format_status"], "passed")
        self.assertEqual(result["structure_status"], "failed")
        self.assertEqual(result["contract_status"], "not_checked")

    def test_bad_json_keeps_independent_plan_and_index_checks(self):
        for raw, code in (
            (b'{"title":', "invalid_json_contract"),
            (b'{"a":1,"a":2,"b":1,"b":2}', "duplicate_json_key"),
            (b'{"value":NaN}', "invalid_json_constant"),
            (b'{"value":"\\ud800"}', "invalid_json_unicode"),
            (b'[]', "json_contract_not_object"),
            (b"\xef\xbb\xbf\xef\xbb\xbf{}", "invalid_json_contract"),
        ):
            with self.subTest(raw=raw):
                self.task_path.write_bytes(raw)
                result = self.diagnose()
                self.assertTrue(any(issue["code"] == code for issue in result["issues"]))
                checks = {item["name"]: item["status"] for item in result["checks"]}
                self.assertEqual(checks["plan"], "passed")
                self.assertEqual(checks["index"], "passed")
                self.assertEqual(checks["structure"], "not_checked")
                self.assertFalse(result["normal_use_allowed"])

    def test_invalid_encoding_is_reported_without_guessing(self):
        self.task_path.write_bytes(self.raw.decode("utf-8").encode("utf-16"))
        result = self.diagnose()
        self.assertEqual(result["format_status"], "failed")
        self.assertEqual(result["raw_sha256"], hashlib.sha256(self.task_path.read_bytes()).hexdigest())
        self.assertEqual(result["structure_status"], "not_checked")
        self.assertTrue(any(issue["code"] == "invalid_utf8" for issue in result["issues"]))

    def test_bom_and_line_endings_are_decodable_but_not_canonical(self):
        for raw in (b"\xef\xbb\xbf" + self.raw, self.raw.replace(b"\n", b"\r\n")):
            with self.subTest(raw=raw[:6]):
                self.task_path.write_bytes(raw)
                result = self.diagnose()
                self.assertEqual(result["structure_status"], "passed")
                self.assertEqual(result["format_status"], "failed")
                self.assertFalse(result["normal_use_allowed"])

    def test_plan_and_index_binding_problems_are_both_reported(self):
        changed = copy.deepcopy(self.contract)
        changed["source_plan"]["canonical_sha256"] = "0" * 64
        self.task_path.write_bytes(render_task_contract(changed))
        self.index["task_spec_id"] = "TASK-SPEC-002"
        self.save_index()
        result = self.diagnose()
        stages = {issue["stage"] for issue in result["issues"]}
        self.assertIn("plan_binding:canonical_sha256", stages)
        self.assertIn("index_binding:task_spec_id", stages)
        self.assertIn("index_binding:task_sha256", stages)

    def test_index_rows_and_instruction_bindings_block_use(self):
        self.index["task_instructions_sha256"] = "0" * 64
        self.index["tasks"][0]["instructions_sha256"] = "1" * 64
        self.save_index()
        result = self.diagnose()
        self.assertEqual(result["contract_status"], "passed")
        self.assertFalse(result["normal_use_allowed"])
        stages = {issue["stage"] for issue in result["issues"]}
        self.assertIn("index_binding:tasks", stages)
        self.assertIn("index_binding:task_instructions_sha256", stages)

    def test_lock_and_active_task_only_block_repair(self):
        self.index["lock"] = {
            "kind": "execution", "task_id": "TASK-001", "attempt_id": "ATTEMPT-001",
            "execute_instructions_sha256": "a" * 64,
        }
        self.index["tasks"][0].update(status="in_progress", latest_attempt="ATTEMPT-001")
        self.index["overall_status"] = "in_progress"
        self.save_index()
        result = self.diagnose()
        self.assertTrue(result["normal_use_allowed"])
        self.assertEqual(result["repair_mode"], "diagnose_only")
        self.assertTrue({"execution_lock", "active_task"} <= {issue["stage"] for issue in result["issues"]})

    def test_pending_transaction_blocks_repair_even_with_unparseable_task(self):
        (self.directory / ".work-spec-update-SPEC-UPDATE-001.json").write_bytes(b"{}")
        self.task_path.write_bytes(b"{")
        result = self.diagnose()
        self.assertEqual(result["repair_mode"], "diagnose_only")
        self.assertTrue(any(issue["code"] == "task_repair_pending_transaction" for issue in result["issues"]))

    def test_persistent_writer_file_is_not_an_active_lock(self):
        (self.directory / ".work-state-writer.lock").write_bytes(b"")
        self.assertEqual(self.diagnose()["repair_mode"], "review_required")

    def test_missing_task_and_index_remain_read_only(self):
        self.task_path.unlink()
        self.index_path.unlink()
        result = self.diagnose()
        self.assertIsNone(result["raw_sha256"])
        self.assertFalse(result["normal_use_allowed"])
        self.assertEqual(result["repair_mode"], "diagnose_only")

    def test_wrong_nested_types_do_not_crash_diagnosis(self):
        cases = [
            ("requirement_id", []), ("tasks", [None]), ("artifacts", 1),
            ("instruction_selection", {"sources": [{"kind": []}]}),
        ]
        for field, value in cases:
            with self.subTest(field=field):
                damaged = copy.deepcopy(self.contract)
                damaged[field] = value
                self.task_path.write_bytes(json.dumps(damaged).encode("utf-8") + b"\n")
                self.assertFalse(self.diagnose()["normal_use_allowed"])

    def test_existing_validator_retains_reason_and_adds_report(self):
        with self.assertRaises(WorkError) as caught:
            validate_task_contract(
                b"{", source="test", actual_task_path=self.artifacts["task"],
                project_root=self.root, user_config_root=str(self.root),
            )
        self.assertEqual(caught.exception.code, "invalid_json_contract")
        self.assertFalse(caught.exception.details["task_diagnostics"]["normal_use_allowed"])

    def test_execute_rejects_invalid_task_before_writer_mutex(self):
        self.task_path.write_bytes(b"{")
        before = self.snapshot()
        with patch("worklib.cli_commands.execute.state_writer") as writer:
            code, envelope = self.cli([
                "execute", "record-begin", "--task-path", self.artifacts["task"],
                "--execution-dir", self.artifacts["execution"], "--task-id", "TASK-001",
                "--record-id", "CMD-001", "--user-config-root", str(self.root),
            ])
        writer.assert_not_called()
        self.assertNotEqual(code, 0)
        self.assertEqual(envelope["reason_code"], "invalid_json_contract")
        self.assertIn("task_diagnostics", envelope["data"])
        self.assertEqual(self.snapshot(), before)

    def test_worktree_rejects_bom_added_after_preflight_before_using_task(self):
        preflight = {
            "task_path": self.artifacts["task"],
            "task_sha256": hashlib.sha256(self.raw).hexdigest(),
        }
        self.task_path.write_bytes(b"\xef\xbb\xbf" + self.raw)
        with patch("worklib.execution.worktree.execute_preflight", return_value=preflight):
            with patch("worklib.execution.worktree.parse_json_contract") as parse:
                with self.assertRaises(WorkError) as caught:
                    inspect_execute_worktree(
                        project_root=self.root, user_config_root=str(self.root),
                        raw_task_path=self.artifacts["task"], raw_execution_dir=self.artifacts["execution"],
                        task_id="TASK-001",
                    )
        self.assertEqual(caught.exception.code, "execute_worktree_task_changed")
        parse.assert_not_called()


if __name__ == "__main__":
    unittest.main()
