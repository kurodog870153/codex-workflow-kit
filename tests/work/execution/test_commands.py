from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.execution.commands import formal_command
from worklib.foundation.errors import WorkError


class CommandTests(unittest.TestCase):
    def test_formal_command_returns_detached_command_value(self) -> None:
        task = {
            "commands": [
                {"id": "CMD-001", "mode": "argv", "argv": ["tool", "old"]}
            ]
        }

        command = formal_command(task, "CMD-001")
        command["argv"].append("changed")

        self.assertEqual(command["mode"], "argv")
        self.assertEqual(task["commands"][0]["argv"], ["tool", "old"])

    def test_formal_command_rejects_unknown_record(self) -> None:
        with self.assertRaises(WorkError) as context:
            formal_command({"commands": []}, "CMD-001")

        self.assertEqual(context.exception.code, "command_correction_command_not_found")


if __name__ == "__main__":
    unittest.main()
