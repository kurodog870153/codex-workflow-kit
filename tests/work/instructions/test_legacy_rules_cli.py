from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import main
from worklib.foundation.errors import ExitCode


class LegacyRulesCliTests(unittest.TestCase):
    def test_rules_resolve_is_not_a_public_command(self) -> None:
        stdout = io.StringIO()
        stderr = io.StringIO()

        exit_code = main(
            [
                "--project-root",
                str(PROJECT_ROOT),
                "rules",
                "resolve",
                "--user-config-root",
                str(PROJECT_ROOT),
                "--work-directory",
                "task",
            ],
            stdout=stdout,
            stderr=stderr,
        )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "cli_usage_error")


if __name__ == "__main__":
    unittest.main()
