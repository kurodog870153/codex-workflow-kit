from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.business_services.task.validation_session import ValidationSession
from worklib.models.common.errors import WorkError
from worklib.services.hierarchy.path import build_hierarchy


class ValidationSessionTests(unittest.TestCase):
    def fixture(self, root: Path) -> Path:
        instruction = root / "references/instructions/task/general/instructions.md"
        instruction.parent.mkdir(parents=True)
        instruction.write_text(
            "---\nname: general\ndescription: General tasks\nmetadata:\n  work-tags: [general]\n---\nBody\n",
            encoding="utf-8",
        )
        workflow = root / "references/workflows/task.md"
        workflow.parent.mkdir(parents=True)
        workflow.write_text("Task workflow\n", encoding="utf-8")
        (root / "references/instruction-loading.md").write_text("Loading\n", encoding="utf-8")
        return instruction

    def test_catalog_change_is_rejected_at_recheck(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            instruction = self.fixture(root)
            session = ValidationSession(root)
            session.catalog("task")
            instruction.write_text(instruction.read_text(encoding="utf-8").replace("General tasks", "Changed tasks"), encoding="utf-8")
            with self.assertRaises(WorkError) as caught:
                session.recheck()
        self.assertEqual(caught.exception.code, "instruction_catalog_changed")

    def test_source_change_is_rejected_at_recheck(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            session = ValidationSession(root)
            hierarchy = build_hierarchy("task", [])
            session.sources("task", hierarchy, [])
            (root / "references/workflows/task.md").write_text("Changed workflow\n", encoding="utf-8")
            with self.assertRaises(WorkError) as caught:
                session.recheck()
        self.assertEqual(caught.exception.code, "instruction_source_changed")


if __name__ == "__main__":
    unittest.main()
