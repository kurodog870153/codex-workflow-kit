from __future__ import annotations

import sys
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts"
WORKLIB_ROOT = SCRIPT_ROOT / "worklib"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.protocol import (
    ATTEMPT_ID_PATTERN,
    BLOCKING_STOPPED_TYPES,
    INVALID_SHA256_ERROR_CODE,
    PLANNING_STATUSES,
    SHA256_PATTERN,
    TASK_ID_PATTERN,
    WORKFLOW_MODES,
)


class ProtocolConstantTests(unittest.TestCase):
    def test_shared_protocol_values_are_stable(self) -> None:
        self.assertEqual(SHA256_PATTERN, r"^[0-9a-f]{64}$")
        self.assertEqual(INVALID_SHA256_ERROR_CODE, "invalid_sha256")
        self.assertEqual(TASK_ID_PATTERN, r"^TASK-\d{3}$")
        self.assertEqual(ATTEMPT_ID_PATTERN, r"^ATTEMPT-\d{3}$")
        self.assertEqual(
            BLOCKING_STOPPED_TYPES,
            frozenset(
                {"external_operation_failed", "instructions_changed", "specification_defect"}
            ),
        )
        self.assertEqual(
            PLANNING_STATUSES,
            ("planned", "in_progress", "refined", "needs_review"),
        )
        self.assertEqual(WORKFLOW_MODES, ("plan", "task", "execute"))

    def test_workflow_modes_are_immutable(self) -> None:
        self.assertIsInstance(WORKFLOW_MODES, tuple)

    def test_canonical_protocol_literals_have_one_product_definition(self) -> None:
        expected_owners = {
            r"^[0-9a-f]{64}$": WORKLIB_ROOT / "protocol" / "shared.py",
            r"^TASK-\d{3}$": WORKLIB_ROOT / "protocol" / "shared.py",
            r"^ATTEMPT-\d{3}$": WORKLIB_ROOT / "protocol" / "shared.py",
            '"invalid_sha256"': WORKLIB_ROOT / "protocol" / "shared.py",
            '("planned", "in_progress", "refined", "needs_review")': (
                WORKLIB_ROOT / "protocol" / "task.py"
            ),
        }
        sources = {
            path: path.read_text(encoding="utf-8")
            for path in WORKLIB_ROOT.rglob("*.py")
        }
        for literal, owner in expected_owners.items():
            occurrences = [path for path, source in sources.items() if literal in source]
            self.assertEqual(occurrences, [owner], literal)


if __name__ == "__main__":
    unittest.main()
