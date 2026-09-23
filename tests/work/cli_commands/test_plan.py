from __future__ import annotations

import io
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.cli import build_parser, main
from worklib.models.common.errors import ExitCode
from artifacts import test_plan as preparation_fixtures


class PlanCliTests(FileInputTestCase):
    def test_semantic_prepare_returns_candidate_and_validation_without_writes(self):
        fixture = preparation_fixtures.InitialPlanPreparationTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        stdout, stderr = io.StringIO(), io.StringIO()
        code = main(self.input_arguments([
            "--project-root", str(fixture.root), "plan", "semantic-prepare",
            "--input-file", "request.json", "--user-config-root", str(fixture.root),
        ], json.dumps(fixture.request)), stdout=stdout, stderr=stderr)
        self.assertEqual(code, ExitCode.SUCCESS, stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        data = json.loads(stdout.getvalue())["data"]
        self.assertEqual(data["schema"], "work-plan-prepare/v1")
        expected = copy.deepcopy(fixture.fixture.contract)
        expected["artifacts"]["task"] = "outputs/work/tasks/example/index.json"
        self.assertEqual(data["plan"], expected)
        self.assertEqual(data["validation"]["schema"], "work-plan-validation/v1")
        self.assertEqual(list(fixture.root.iterdir()), [])

    def test_semantic_prepare_output_file_preserves_unicode_canonical_plan_and_never_overwrites(self):
        fixture = preparation_fixtures.InitialPlanPreparationTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        fixture.request["title"] = "跨平台計畫"
        output_path = fixture.root / "transaction" / "plan-candidate.json"
        output_path.parent.mkdir()
        arguments = [
            "--project-root", str(fixture.root), "plan", "semantic-prepare",
            "--input-file", "request.json", "--user-config-root", str(fixture.root),
            "--output-file", str(output_path),
        ]
        stdout, stderr = io.StringIO(), io.StringIO()
        code = main(self.input_arguments(arguments, json.dumps(fixture.request)), stdout=stdout, stderr=stderr)
        self.assertEqual((code, stderr.getvalue()), (ExitCode.SUCCESS, ""))
        data = json.loads(stdout.getvalue())["data"]
        raw = output_path.read_bytes()
        self.assertFalse(raw.startswith(b"\xef\xbb\xbf"))
        self.assertNotIn(b"\r\n", raw)
        self.assertEqual(json.loads(raw.decode("utf-8")), data["plan"])

        stdout = io.StringIO()
        code = main(self.input_arguments(arguments, json.dumps(fixture.request)), stdout=stdout, stderr=io.StringIO())
        self.assertEqual(code, ExitCode.WORKFLOW_STATE)
        self.assertEqual(json.loads(stdout.getvalue())["reason_code"], "plan_prepare_output_exists")

    def test_old_prepare_command_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            stdout = io.StringIO()
            code = main(["--project-root", directory, "plan", "prepare",
                "--input-file", "request.json", "--user-config-root", directory],
                stdout=stdout, stderr=io.StringIO())
        self.assertEqual(code, ExitCode.CLI_USAGE)

    def test_validate_input_file_arguments_parse(self) -> None:
        arguments = build_parser().parse_args(
            [
                "--project-root",
                "/project",
                "plan",
                "validate",
                "--user-config-root",
                "/config",
                "--skill-root",
                "repo:.agents/skills=/skills",
                "--input-file", "request.json",
                "--plan-path",
                "outputs/work/plans/example.json",
            ]
        )

        self.assertEqual(arguments.command, "plan")
        self.assertEqual(arguments.plan_command, "validate")
        self.assertEqual(arguments.project_root, "/project")
        self.assertEqual(arguments.user_config_root, "/config")
        self.assertEqual(
            arguments.skill_root,
            ["repo:.agents/skills=/skills"],
        )
        self.assertTrue(arguments.input_file)
        self.assertIsNone(arguments.path)
        self.assertEqual(
            arguments.plan_path,
            "outputs/work/plans/example.json",
        )

    def test_validate_input_file_requires_plan_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                self.input_arguments([
                    "--project-root",
                    project_directory,
                    "plan",
                    "validate",
                    "--user-config-root",
                    project_directory,
                    "--input-file", "request.json",
                ], "{}"),
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stderr.getvalue(), "")
        error = json.loads(stdout.getvalue())
        self.assertEqual(error["schema"], "work-cli-result/v1")
        self.assertEqual(error["reason_code"], "plan_path_required")

    def test_validate_file_rejects_plan_path(self) -> None:
        with tempfile.TemporaryDirectory() as project_directory:
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                [
                    "--project-root",
                    project_directory,
                    "plan",
                    "validate",
                    "--user-config-root",
                    project_directory,
                    "--path",
                    "outputs/work/plans/example.json",
                    "--plan-path",
                    "outputs/work/plans/other.json",
                ],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stderr.getvalue(), "")
        error = json.loads(stdout.getvalue())
        self.assertEqual(error["schema"], "work-cli-result/v1")
        self.assertEqual(error["reason_code"], "unexpected_plan_path")


if __name__ == "__main__":
    unittest.main()
