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


class InstructionCatalogCliTests(unittest.TestCase):
    def test_catalog_command_discovers_each_installed_mode(self) -> None:
        expected_paths = {
            "plan": (
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/frontend",
                "web/frontend/css",
                "web/frontend/typescript",
            ),
            "task": (
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
                "web/frontend",
                "web/frontend/css",
                "web/frontend/css/tailwind",
                "web/frontend/typescript",
                "web/frontend/typescript/astro",
            ),
            "execute": (
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
                "web/frontend",
                "web/frontend/css",
                "web/frontend/css/tailwind",
                "web/frontend/typescript",
                "web/frontend/typescript/astro",
            ),
        }
        with tempfile.TemporaryDirectory() as project_directory:
            for mode, paths in expected_paths.items():
                with self.subTest(mode=mode):
                    stdout = io.StringIO()
                    stderr = io.StringIO()

                    exit_code = main(
                        [
                            "--project-root",
                            project_directory,
                            "instructions",
                            "catalog",
                            "--mode",
                            mode,
                        ],
                        stdout=stdout,
                        stderr=stderr,
                    )

                    self.assertEqual(exit_code, ExitCode.SUCCESS)
                    self.assertEqual(stderr.getvalue(), "")
                    result = json.loads(stdout.getvalue())
                    self.assertEqual(result["schema"], "work-instruction-catalog/v1")
                    self.assertEqual(result["mode"], mode)
                    self.assertEqual(tuple(result["paths"]), paths)
                    self.assertEqual(result["children"]["general"], ["web"])
                    self.assertEqual(
                        set(result["metadata"]["general"]),
                        {"name", "description", "work_tags"},
                    )
                    self.assertNotIn("指令邊界", str(result["metadata"]))

    def test_invalid_mode_is_reported_as_cli_usage(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "catalog",
                    "--mode",
                    "build",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "cli_usage_error")

    def test_catalog_command_combines_all_modes(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "catalog",
                    "--mode",
                    "all",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["schema"], "work-instruction-catalog/v1")
        self.assertEqual(result["mode"], "all")
        self.assertEqual(
            result["metadata"]["web/backend/java/jpa"]["mode_support"],
            ["task", "execute"],
        )
        self.assertRegex(result["catalog_sha256"], r"^[0-9a-f]{64}$")

    def test_resolve_command_expands_ordered_leaf_paths(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "resolve",
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
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["schema"], "work-hierarchy/v1")
        self.assertEqual(
            result["selected_paths"],
            ["web/backend/java/jpa", "web/backend/java/mybatis"],
        )
        self.assertEqual(
            result["resolved_paths"],
            [
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
            ],
        )

    def test_resolve_command_reports_valid_choices_for_missing_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "resolve",
                    "--mode",
                    "task",
                    "web/backend/java/hibernate",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CONTRACT)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["code"], "instruction_hierarchy_path_missing")
        self.assertEqual(
            error["details"],
            {
                "mode": "task",
                "parent": "web/backend/java",
                "path": "web/backend/java/hibernate",
                "valid_choices": ["jpa", "mybatis"],
            },
        )

    def test_plan_resolve_preserves_leaf_and_projects_loaded_paths(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "instructions",
                    "resolve",
                    "--mode",
                    "plan",
                    "web/backend/java/jpa",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["selected_paths"], ["web/backend/java/jpa"])
        self.assertEqual(
            result["resolved_paths"],
            ["general", "web", "web/backend", "web/backend/java"],
        )


if __name__ == "__main__":
    unittest.main()
