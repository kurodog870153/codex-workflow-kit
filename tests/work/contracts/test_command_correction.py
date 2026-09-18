from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.services.attempt.validation import canonicalize_command_correction
from worklib.models.common.errors import WorkError


class CommandCorrectionContractTests(unittest.TestCase):
    def correction(self) -> dict[str, object]:
        return {
            "original_command": {"mode": "argv", "argv": ["tool", "old"]},
            "actual_command": {"mode": "argv", "argv": ["tool", "new"]},
            "reason": "Use the authorized argument.",
            "authorization_evidence": "Manifest authorization.",
        }

    def test_canonicalizes_argv_commands(self) -> None:
        correction = self.correction()

        self.assertEqual(canonicalize_command_correction(correction), correction)

    def test_canonicalizes_shell_commands(self) -> None:
        correction = self.correction()
        correction["original_command"] = {"mode": "shell", "script": "tool old"}
        correction["actual_command"] = {"mode": "shell", "script": "tool new"}

        self.assertEqual(canonicalize_command_correction(correction), correction)

    def test_rejects_mode_change(self) -> None:
        correction = self.correction()
        correction["actual_command"] = {"mode": "shell", "script": "tool new"}

        with self.assertRaises(WorkError) as context:
            canonicalize_command_correction(correction)

        self.assertEqual(
            context.exception.code,
            "attempt_command_correction_mode_mismatch",
        )

    def test_rejects_unchanged_command(self) -> None:
        correction = self.correction()
        correction["actual_command"] = correction["original_command"]

        with self.assertRaises(WorkError) as context:
            canonicalize_command_correction(correction)

        self.assertEqual(context.exception.code, "attempt_command_correction_unchanged")

    def test_rejects_missing_and_unknown_fields(self) -> None:
        correction = self.correction()
        correction.pop("reason")
        correction["extra"] = True

        with self.assertRaises(WorkError) as context:
            canonicalize_command_correction(correction, location="lock.correction")

        self.assertEqual(context.exception.code, "attempt_invalid_object_fields")
        self.assertEqual(context.exception.details["location"], "lock.correction")
        self.assertEqual(context.exception.details["missing"], ["reason"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])


if __name__ == "__main__":
    unittest.main()
