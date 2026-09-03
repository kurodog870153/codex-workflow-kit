from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import main
from worklib.foundation.errors import ExitCode


class InstructionSourcesCliTests(unittest.TestCase):
    def test_load_command_returns_ordered_installed_sources(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "load",
                    "--mode",
                    "task",
                    "--reference",
                    "task.web.backend.security",
                    "--reference",
                    "task.general.task-records",
                    "web/backend",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["schema"], "work-instructions/v1")
        self.assertEqual(result["mode"], "task")
        self.assertEqual(
            [source["logical_name"] for source in result["sources"]],
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.general.task-records",
                "task.web",
                "task.web.backend",
                "task.web.backend.security",
            ],
        )
        self.assertEqual(
            result["references"],
            ["task.general.task-records", "task.web.backend.security"],
        )
        self.assertEqual(len(result["instructions_sha256"]), 64)
        for source in result["sources"]:
            self.assertEqual(
                set(source),
                {"kind", "logical_name", "canonical_sha256"},
            )

    def test_load_command_reports_unroutable_reference(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "load",
                    "--mode",
                    "task",
                    "--reference",
                    "task.web.backend.security",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CONTRACT)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "unroutable_instruction_reference")
        self.assertEqual(
            error["details"]["logical_name"],
            "task.web.backend.security",
        )

    def test_select_command_returns_canonical_selection(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "select",
                    "--mode",
                    "task",
                    "--reference",
                    "task.web.backend.security",
                    "--reference",
                    "task.general.task-records",
                    "web/backend",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["schema"], "work-instruction-selection/v1")
        self.assertEqual(result["mode"], "task")
        selection = result["instruction_selection"]
        self.assertEqual(
            list(selection),
            [
                "instructions_sha256",
                "references",
                "resolved_paths",
                "selected_paths",
                "sources",
            ],
        )
        self.assertEqual(
            selection["references"],
            ["task.general.task-records", "task.web.backend.security"],
        )
        self.assertTrue(
            all("layer" not in source for source in selection["sources"])
        )

    def test_select_command_preserves_ordered_leaf_paths(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "select",
                    "--mode",
                    "task",
                    "web/backend/java/jpa",
                    "web/backend/java/mybatis",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        selection = json.loads(stdout.getvalue())["instruction_selection"]
        self.assertEqual(
            selection["selected_paths"],
            ["web/backend/java/jpa", "web/backend/java/mybatis"],
        )

    def test_select_command_reports_unroutable_reference(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "select",
                    "--mode",
                    "task",
                    "--reference",
                    "task.web.backend.security",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CONTRACT)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["code"], "unroutable_instruction_reference")


if __name__ == "__main__":
    unittest.main()
