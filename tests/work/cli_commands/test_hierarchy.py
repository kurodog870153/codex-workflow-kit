from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import main
from worklib.foundation.errors import ExitCode


class HierarchyCliTests(unittest.TestCase):
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
            [
                "--project-root",
                project_root,
                "hierarchy",
                "selection-build",
                "--stdin",
            ],
            stdin=io.StringIO(json.dumps(request)),
            stdout=stdout,
            stderr=stderr,
        )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        selection = json.loads(stdout.getvalue())
        validate_stdout = io.StringIO()
        validate_stderr = io.StringIO()
        validate_exit_code = main(
            [
                "--project-root",
                project_root,
                "hierarchy",
                "selection-validate",
                "--stdin",
            ],
            stdin=io.StringIO(json.dumps(selection)),
            stdout=validate_stdout,
            stderr=validate_stderr,
        )

        self.assertEqual(validate_exit_code, ExitCode.SUCCESS)
        self.assertEqual(validate_stderr.getvalue(), "")
        self.assertEqual(json.loads(validate_stdout.getvalue())["status"], "valid")

if __name__ == "__main__":
    unittest.main()
