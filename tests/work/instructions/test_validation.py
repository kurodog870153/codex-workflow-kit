from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.instructions.validation import sha256, strict_object, string_array


class InstructionValidationTests(unittest.TestCase):
    def test_strict_object_returns_object_with_exact_fields(self) -> None:
        value = {"name": "work"}

        self.assertIs(
            strict_object(value, location="selection", required={"name"}),
            value,
        )

    def test_strict_object_reports_missing_and_unknown_fields(self) -> None:
        with self.assertRaises(WorkError) as context:
            strict_object(
                {"extra": True},
                location="selection",
                required={"name"},
            )

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["name"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])

    def test_string_array_rejects_empty_item_at_precise_location(self) -> None:
        with self.assertRaises(WorkError) as context:
            string_array(["general", ""], location="paths", allow_empty=True)

        self.assertEqual(context.exception.code, "invalid_string_array")
        self.assertEqual(context.exception.details["location"], "paths[1]")

    def test_sha256_accepts_lowercase_digest_and_rejects_uppercase(self) -> None:
        digest = "a" * 64
        self.assertEqual(sha256(digest, location="fingerprint"), digest)

        with self.assertRaises(WorkError) as context:
            sha256("A" * 64, location="fingerprint")

        self.assertEqual(context.exception.code, "invalid_sha256")


if __name__ == "__main__":
    unittest.main()
