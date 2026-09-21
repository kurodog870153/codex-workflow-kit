from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


TEST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_ROOT))
sys.path.insert(0, str(TEST_ROOT.parents[1] / "skills" / "work" / "scripts"))

from contracts import test_task_collection
from worklib.cli import main
from worklib.services.attempt.validation import (
    build_initial_execution_index,
    render_execution_index,
)
from worklib.workflows.task import diagnose_task_collection
from worklib.business_services.task.index import render_task_index_contract
from worklib.workflows.execution import ExecutionOperations, inspect_execute_worktree
from worklib.models.common.errors import WorkError
from worklib.technical.foundation.fingerprint import raw_sha256
from worklib.business_services.task import load_task_collection


class TaskCollectionDiagnosticsTests(unittest.TestCase):
    def test_incomplete_migration_transaction_blocks_repair_mode(self) -> None:
        execution = self.root / self.artifacts["execution"]
        (execution / ".work-spec-migration-ABC.json").write_bytes(b"{}\n")
        result = diagnose_task_collection(
            self.root, str(self.root), self.task_path,
        )
        self.assertTrue(any(issue["code"] == "task_repair_pending_transaction" for issue in result["issues"]))

    def setUp(self) -> None:
        fixture = test_task_collection.TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.root = fixture.root
        self.task_path = fixture.index_path
        self.index_path = self.root / self.task_path
        self.item_path = self.index_path.parent / "tasks/TASK-001.json"
        self.raw_index = self.index_path.read_bytes()
        self.raw_item = self.item_path.read_bytes()
        self.validation = load_task_collection(
            self.root, str(self.root), self.task_path
        )
        self.contract = self.validation["collection_contract"]
        self.artifacts = self.contract["artifacts"]
        self.directory = self.root / self.artifacts["execution"]
        self.directory.mkdir(parents=True, exist_ok=True)
        self.execution_path = self.directory / "index.json"
        self.execution = build_initial_execution_index(
            self.contract, self.validation
        )
        self.save_execution()

    def save_execution(self) -> None:
        self.execution_path.write_bytes(render_execution_index(self.execution))

    def snapshot(self) -> dict[str, bytes | None]:
        return {
            path.relative_to(self.root).as_posix(): (
                path.read_bytes() if path.is_file() else None
            )
            for path in self.root.rglob("*")
        }

    def diagnose(self) -> dict[str, object]:
        before = self.snapshot()
        result = diagnose_task_collection(
            self.root, str(self.root), self.task_path
        )
        self.assertEqual(self.snapshot(), before)
        json.dumps(result, allow_nan=False)
        return result

    def cli(self, arguments: list[str]) -> tuple[int, dict[str, object]]:
        output, errors = io.StringIO(), io.StringIO()
        code = main(
            ["--project-root", str(self.root), *arguments],
            stdout=output,
            stderr=errors,
        )
        self.assertEqual(errors.getvalue(), "")
        return code, json.loads(output.getvalue())

    def test_valid_report_and_cli_are_read_only(self) -> None:
        result = self.diagnose()
        self.assertTrue(result["normal_use_allowed"])
        self.assertEqual(result["contract_status"], "passed")
        self.assertEqual(result["execution_binding_status"], "passed")
        before = self.snapshot()
        code, envelope = self.cli([
            "task", "diagnose", "--path", self.task_path,
            "--plan-path", self.artifacts["plan"],
            "--execution-dir", self.artifacts["execution"],
            "--user-config-root", str(self.root),
        ])
        self.assertEqual(code, 0)
        self.assertEqual(envelope["data"], result)
        self.assertEqual(self.snapshot(), before)

    def test_invalid_index_json_blocks_collection_without_writes(self) -> None:
        for raw, code in (
            (b'{"title":', "invalid_json_contract"),
            (b'{"a":1,"a":2}', "duplicate_json_key"),
            (b'{"value":NaN}', "invalid_json_constant"),
            (b'[]', "json_contract_not_object"),
        ):
            with self.subTest(code=code):
                self.index_path.write_bytes(raw)
                result = self.diagnose()
                self.assertTrue(
                    any(issue["code"] == code for issue in result["issues"])
                )
                self.assertFalse(result["normal_use_allowed"])
                self.index_path.write_bytes(self.raw_index)

    def test_invalid_item_keeps_independent_index_and_plan_checks(self) -> None:
        self.item_path.write_bytes(b"{")
        result = self.diagnose()
        checks = {item["name"]: item["status"] for item in result["checks"]}
        self.assertEqual(checks["index:contract"], "passed")
        self.assertEqual(checks["plan:contract"], "passed")
        self.assertEqual(checks["item:TASK-001:json"], "failed")
        self.assertFalse(result["normal_use_allowed"])

    def test_item_encoding_and_normalization_are_reported(self) -> None:
        for raw, stage in (
            (self.raw_item.decode("utf-8").encode("utf-16"), "item:TASK-001:encoding"),
            (b"\xef\xbb\xbf" + self.raw_item, "item:TASK-001:normalization"),
            (self.raw_item.replace(b"\n", b"\r\n"), "item:TASK-001:normalization"),
        ):
            with self.subTest(stage=stage):
                self.item_path.write_bytes(raw)
                result = self.diagnose()
                checks = {item["name"]: item["status"] for item in result["checks"]}
                self.assertEqual(checks[stage], "failed")
                self.assertFalse(result["normal_use_allowed"])
                self.item_path.write_bytes(self.raw_item)

    def test_item_contract_and_fingerprint_problems_are_reported(self) -> None:
        item = json.loads(self.raw_item)
        del item["goal"]
        item["validations"] = []
        self.item_path.write_bytes(json.dumps(item).encode() + b"\n")
        result = self.diagnose()
        self.assertEqual(result["contract_status"], "not_checked")
        self.assertFalse(result["normal_use_allowed"])

        self.item_path.write_bytes(self.raw_item)
        index = json.loads(self.raw_index)
        index["tasks"][0]["canonical_sha256"] = "0" * 64
        self.index_path.write_bytes(render_task_index_contract(index))
        result = self.diagnose()
        self.assertTrue(
            any(
                issue["code"] == "task_item_fingerprint_mismatch"
                for issue in result["issues"]
            )
        )

    def test_plan_and_execution_bindings_are_checked(self) -> None:
        index = json.loads(self.raw_index)
        index["source_plan"]["canonical_sha256"] = "0" * 64
        self.index_path.write_bytes(render_task_index_contract(index))
        result = self.diagnose()
        self.assertTrue(
            any(
                issue["code"] == "source_plan_fingerprint_mismatch"
                for issue in result["issues"]
            )
        )

        self.index_path.write_bytes(self.raw_index)
        self.execution["task_spec_id"] = "TASK-SPEC-999"
        self.save_execution()
        result = self.diagnose()
        self.assertEqual(result["execution_binding_status"], "failed")
        self.assertTrue(
            any(
                issue["code"] == "execution_task_binding_mismatch"
                for issue in result["issues"]
            )
        )

    def test_missing_index_or_item_remains_read_only(self) -> None:
        self.item_path.unlink()
        result = self.diagnose()
        self.assertFalse(result["normal_use_allowed"])
        self.assertTrue(any(issue["code"] == "file_not_found" for issue in result["issues"]))

        self.index_path.unlink()
        result = self.diagnose()
        self.assertIsNone(result["raw_sha256"])
        self.assertFalse(result["normal_use_allowed"])

    def test_missing_execution_index_is_reported_without_blocking_collection(self) -> None:
        self.execution_path.unlink()

        result = self.diagnose()
        checks = {item["name"]: item["status"] for item in result["checks"]}

        self.assertTrue(result["normal_use_allowed"])
        self.assertEqual(checks["collection:contract"], "passed")
        self.assertEqual(checks["execution:file"], "failed")
        self.assertEqual(checks["execution:contract"], "not_checked")
        self.assertEqual(result["execution_binding_status"], "not_checked")

    def test_orphan_and_broken_item_are_both_reported(self) -> None:
        self.item_path.write_bytes(b"{")
        (self.item_path.parent / "TASK-999.json").write_bytes(b"{}\n")

        result = self.diagnose()
        checks = {item["name"]: item["status"] for item in result["checks"]}
        issue_codes = {issue["code"] for issue in result["issues"]}

        self.assertEqual(checks["item:TASK-001:json"], "failed")
        self.assertEqual(checks["items:directory"], "failed")
        self.assertIn("invalid_json_contract", issue_codes)
        self.assertIn("task_collection_directory_mismatch", issue_codes)
        self.assertEqual(checks["collection:contract"], "not_checked")

    def test_execute_rejects_invalid_item_before_writer_mutex(self) -> None:
        self.item_path.write_bytes(b"{")
        before = self.snapshot()
        with patch(
            "worklib.business_services.execution.workflow.state_writer"
        ) as writer:
            code, envelope = self.cli([
                "execute", "record-begin", "--task-path", self.task_path,
                "--execution-dir", self.artifacts["execution"],
                "--task-id", "TASK-001", "--record-id", "CMD-001",
                "--user-config-root", str(self.root),
            ])
        writer.assert_not_called()
        self.assertNotEqual(code, 0)
        self.assertEqual(envelope["reason_code"], "invalid_json_contract")
        self.assertEqual(set(envelope["data"]), {"line", "column"})
        self.assertEqual(self.snapshot(), before)

    def test_worktree_rejects_item_change_after_preflight(self) -> None:
        preflight = {
            "task_path": self.task_path,
            "task_index_sha256": raw_sha256(self.raw_index),
            "task_item_sha256": raw_sha256(self.raw_item),
        }
        self.item_path.write_bytes(b"\xef\xbb\xbf" + self.raw_item)
        with patch(
            "worklib.business_services.execution.worktree.execute_preflight",
            return_value=preflight,
        ), patch.object(ExecutionOperations, "load_task_execution_context") as load:
            with self.assertRaises(WorkError) as caught:
                inspect_execute_worktree(
                    project_root=self.root,
                    user_config_root=str(self.root),
                    raw_task_path=self.task_path,
                    raw_execution_dir=self.artifacts["execution"],
                    task_id="TASK-001",
                )
        self.assertEqual(caught.exception.code, "execute_worktree_task_changed")
        load.assert_not_called()


if __name__ == "__main__":
    unittest.main()
