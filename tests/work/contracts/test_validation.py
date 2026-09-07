from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.validation import nonempty_string, sha256, strict_keys
from worklib.foundation.errors import WorkError


class ContractValidationTests(unittest.TestCase):
    def test_strict_keys_returns_object_with_required_and_optional_fields(self) -> None:
        value = {"id": "ITEM-001", "note": "Optional."}

        self.assertIs(
            strict_keys(
                value,
                location="item",
                required={"id"},
                optional={"note"},
            ),
            value,
        )

    def test_strict_keys_rejects_nonobject(self) -> None:
        with self.assertRaises(WorkError) as context:
            strict_keys([], location="item", required={"id"})

        self.assertEqual(context.exception.code, "expected_object")
        self.assertEqual(context.exception.details["location"], "item")

    def test_strict_keys_reports_missing_and_unknown_fields(self) -> None:
        with self.assertRaises(WorkError) as context:
            strict_keys(
                {"extra": True},
                location="item",
                required={"id"},
            )

        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["missing"], ["id"])
        self.assertEqual(context.exception.details["unknown"], ["extra"])

    def test_nonempty_string_rejects_blank_value(self) -> None:
        with self.assertRaises(WorkError) as context:
            nonempty_string(" ", location="item.name")

        self.assertEqual(context.exception.code, "empty_text_value")
        self.assertEqual(context.exception.details["location"], "item.name")

    def test_sha256_accepts_lowercase_and_rejects_uppercase(self) -> None:
        digest = "a" * 64
        self.assertEqual(sha256(digest, location="fingerprint"), digest)

        with self.assertRaises(WorkError) as context:
            sha256("A" * 64, location="fingerprint")

        self.assertEqual(context.exception.code, "invalid_sha256")
        self.assertEqual(context.exception.details["location"], "fingerprint")


if __name__ == "__main__":
    unittest.main()
