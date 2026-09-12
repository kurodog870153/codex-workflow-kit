from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.cli import main
from worklib.foundation.errors import ExitCode


class HierarchyCliTests(FileInputTestCase):
    def test_cli_build_and_validate_round_trip(self) -> None:
        project_root = str(Path(__file__).resolve().parents[3])
        stdout = io.StringIO()
        stderr = io.StringIO()
        request = {
            "decision": "instruction_paths",
            "selections": [
                {
                    "path": "web/backend/java/jpa",
                    "recommendation_reason": "The work uses JPA persistence.",
                }
            ],
        }

        exit_code = main(
            self.input_arguments([
                "--project-root",
                project_root,
                "hierarchy",
                "selection-build",
                "--input-file", "request.json",
            ], json.dumps(request)),
            stdout=stdout,
            stderr=stderr,
        )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        selection = json.loads(stdout.getvalue())["data"]
        validate_stdout = io.StringIO()
        validate_stderr = io.StringIO()
        validate_exit_code = main(
            self.input_arguments([
                "--project-root",
                project_root,
                "hierarchy",
                "selection-validate",
                "--input-file", "request.json",
            ], json.dumps(selection)),
            stdout=validate_stdout,
            stderr=validate_stderr,
        )

        self.assertEqual(validate_exit_code, ExitCode.SUCCESS)
        self.assertEqual(validate_stderr.getvalue(), "")
        self.assertEqual(json.loads(validate_stdout.getvalue())["data"]["status"], "valid")

if __name__ == "__main__":
    unittest.main()
