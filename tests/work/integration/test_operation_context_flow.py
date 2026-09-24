from __future__ import annotations

import argparse
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.models.common.errors import WorkError
from worklib.business_services.workflow import (
    build_cli_operation_context, execute_with_operation_context, validate_operation_context,
)
from worklib.technical.infrastructure.cli_io import read_input_file
from worklib.business_services.workflow import operation


SKILL_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work"


class OperationContextFlowTests(unittest.TestCase):
    def arguments(self, root: Path) -> argparse.Namespace:
        request = root / "request.json"
        request.write_text("{}\n", encoding="utf-8")
        return argparse.Namespace(
            command="progress", progress_command="read", requirement_id="example",
            mode="plan", input_file=str(request), approved_sha256=None,
        )

    def test_main_flow_builds_validates_consumes_and_closes_one_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            arguments = self.arguments(root)
            calls = []
            result = execute_with_operation_context(
                arguments, root, SKILL_ROOT,
                lambda: calls.append("worker") or {
                    "schema": "work-progress-read/v1", "status": "saved",
                },
            )
        self.assertEqual(calls, ["worker"])
        self.assertEqual(result["schema"], "work-progress-read/v1")

    def test_context_rechecks_bindings_once_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            arguments = self.arguments(root)
            with patch.object(operation, "_artifact_bindings", wraps=operation._artifact_bindings) as bindings:
                execute_with_operation_context(
                    arguments, root, SKILL_ROOT,
                    lambda: {"schema": "work-progress-read/v1", "status": "saved"},
                )
        self.assertEqual(bindings.call_count, 2)

    def test_pre_read_request_is_bound_and_changed_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            arguments = self.arguments(root)
            request = read_input_file(arguments.input_file)
            Path(arguments.input_file).write_text('{"changed":true}\n', encoding="utf-8")
            with self.assertRaises(WorkError) as caught:
                execute_with_operation_context(
                    arguments, root, SKILL_ROOT,
                    lambda: self.fail("worker must not run"), request=request,
                )
        self.assertEqual(caught.exception.code, "operation_artifact_drift")

    def test_artifact_drift_is_rejected_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            arguments = self.arguments(root)
            envelope, routing = build_cli_operation_context(arguments, root, SKILL_ROOT)
            Path(arguments.input_file).write_text('{"changed":true}\n', encoding="utf-8")
            with self.assertRaises(WorkError) as caught:
                validate_operation_context(envelope, routing, arguments, project_root=root)
        self.assertEqual(caught.exception.code, "operation_artifact_drift")

    def test_bound_field_change_invalidates_context_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            arguments = self.arguments(root)
            envelope, routing = build_cli_operation_context(arguments, root, SKILL_ROOT)
            envelope["role"] = "worker"
            with self.assertRaises(WorkError) as caught:
                validate_operation_context(envelope, routing, arguments, project_root=root)
        self.assertEqual(caught.exception.code, "operation_context_identity_mismatch")

    def test_operation_change_creates_a_new_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            read_arguments = self.arguments(root)
            read_envelope, _ = build_cli_operation_context(read_arguments, root, SKILL_ROOT)
            save_arguments = self.arguments(root)
            save_arguments.progress_command = "save"
            save_arguments.approved_sha256 = "a" * 64
            save_envelope, _ = build_cli_operation_context(save_arguments, root, SKILL_ROOT)
        self.assertNotEqual(read_envelope["context_sha256"], save_envelope["context_sha256"])
        self.assertEqual(read_envelope["authorization_state"], "read_only")
        self.assertEqual(save_envelope["authorization_state"], "authorized")
        self.assertEqual(save_envelope["side_effect_boundary"], "authorized_atomic_write")

    def test_project_artifacts_resolve_from_project_root_and_input_uses_cwd(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            request = root / "request.json"
            request.write_text("{}\n", encoding="utf-8")
            arguments = argparse.Namespace(
                command="execute", execute_command="deviation-record",
                task_path="outputs/work/tasks/example/index.json",
                execution_dir="outputs/work/executions/example", input_file=str(request),
                approved_sha256="a" * 64, authorization_evidence="approved",
                user_config_root=str(root), task_id="TASK-001", skill_root=[],
            )
            envelope, _ = build_cli_operation_context(arguments, root, SKILL_ROOT)
        self.assertEqual(envelope["artifacts"]["task_path"]["path"],
                         str((root / "outputs/work/tasks/example/index.json").resolve()))
        self.assertEqual(envelope["artifacts"]["input_file"]["path"], str(request.resolve()))

    def test_command_run_is_explicit_external_effect(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            request = root / "request.json"
            request.write_text("{}\n", encoding="utf-8")
            arguments = argparse.Namespace(
                command="execute", execute_command="command-run",
                task_path="outputs/work/tasks/example/index.json",
                execution_dir="outputs/work/executions/example", input_file=str(request),
                approved_sha256="a" * 64, user_config_root=str(root), task_id="TASK-001", skill_root=[],
            )
            envelope, _ = build_cli_operation_context(arguments, root, SKILL_ROOT)
        self.assertEqual(envelope["side_effect_boundary"], "authorized_external_effect")

    def test_recovery_event_uses_a_new_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            request = root / "request.json"
            request.write_text("{}\n", encoding="utf-8")
            common = dict(
                command="execute", user_config_root=str(root), task_path="request.json",
                execution_dir="execution", task_id="TASK-001",
                skill_root=[], input_file=str(request), approved_sha256=None,
            )
            normal = argparse.Namespace(execute_command="command-prepare", **common)
            recovery = argparse.Namespace(execute_command="recover", **common)
            normal_envelope, _ = build_cli_operation_context(normal, root, SKILL_ROOT)
            recovery_envelope, recovery_routing = build_cli_operation_context(recovery, root, SKILL_ROOT)
        self.assertNotEqual(normal_envelope["context_sha256"], recovery_envelope["context_sha256"])
        self.assertIn("event:recovery", recovery_routing["routing_reasons"])


if __name__ == "__main__":
    unittest.main()
