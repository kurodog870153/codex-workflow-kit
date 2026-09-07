from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import ExitCode, WorkError


class WorkErrorTests(unittest.TestCase):
    def test_preserves_error_attributes(self) -> None:
        details = {"location": "task.id"}

        error = WorkError(
            ExitCode.CONTRACT,
            "invalid_task_id",
            "The task identifier is invalid.",
            details,
        )

        self.assertEqual(error.exit_code, ExitCode.CONTRACT)
        self.assertEqual(error.code, "invalid_task_id")
        self.assertEqual(str(error), "The task identifier is invalid.")
        self.assertIs(error.details, details)

    def test_serializes_stable_error_contract(self) -> None:
        error = WorkError(
            ExitCode.IO_FAILURE,
            "read_failed",
            "The file could not be read.",
        )

        self.assertEqual(
            error.as_dict(),
            {
                "schema": "work-error/v1",
                "code": "read_failed",
                "message": "The file could not be read.",
                "details": {},
            },
        )


if __name__ == "__main__":
    unittest.main()
