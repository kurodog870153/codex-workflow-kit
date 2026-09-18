from __future__ import annotations

import hashlib
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.technical.foundation.fingerprint import instructions_sha256


def expected_sha256(
    scope: str,
    sources: list[tuple[str, str, bytes]],
) -> str:
    framed = bytearray(b"WORK-INSTRUCTIONS-SHA-256-V1\n")
    for kind, logical_name, content in sources:
        framed.extend(b"S")
        for value in (
            scope.encode("utf-8"),
            kind.encode("utf-8"),
            logical_name.encode("utf-8"),
            content,
        ):
            framed.extend(str(len(value)).encode("ascii"))
            framed.extend(b":")
            framed.extend(value)
        framed.extend(b"\n")
    framed.extend(b"END\n")
    return hashlib.sha256(framed).hexdigest()


class InstructionFingerprintTests(unittest.TestCase):
    def setUp(self) -> None:
        self.sources = [
            ("workflow", "work.instruction-loading", b"loading\n"),
            ("workflow", "work.workflow.task", b"workflow\n"),
            ("instruction", "task.general", b"general\n"),
            ("reference", "task.general.task-records", b"records\n"),
        ]

    def test_exact_frame_is_deterministic_and_has_no_layer(self) -> None:
        expected = expected_sha256("task", self.sources)
        self.assertEqual(instructions_sha256("task", self.sources), expected)
        self.assertEqual(instructions_sha256("task", self.sources), expected)

    def test_source_order_changes_fingerprint(self) -> None:
        reordered = [self.sources[0], self.sources[2], self.sources[1], self.sources[3]]
        self.assertNotEqual(
            instructions_sha256("task", self.sources),
            instructions_sha256("task", reordered),
        )

    def test_all_source_kinds_are_accepted(self) -> None:
        for kind in ("workflow", "instruction", "reference"):
            with self.subTest(kind=kind):
                digest = instructions_sha256("plan", [(kind, f"test.{kind}", b"x\n")])
                self.assertEqual(len(digest), 64)

if __name__ == "__main__":
    unittest.main()
