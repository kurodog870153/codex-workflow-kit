from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.services.progress.prepare import prepare_progress


class ProgressServiceUnitTests(unittest.TestCase):
    def test_prepare_uses_injected_validation_and_preview(self) -> None:
        calls: list[str] = []
        content = {
            "title": "Title", "request": "Request", "current_task_id": None,
            "context": {}, "source_status": [], "notes": [],
            "confirmed_decisions": [], "tentative": [], "open_questions": [],
            "next_discussion_point": "Continue",
        }

        def validate_keys(value, **kwargs):
            calls.append("validate")
            return value

        def preview(root, value, **kwargs):
            calls.append("preview")
            return {"schema": "work-progress-preview/v1", "progress": value}

        with tempfile.TemporaryDirectory() as temporary:
            result = prepare_progress(
                Path(temporary), content, requirement_id="example", mode="plan",
                expected_revision=0, preview=preview, validate_keys=validate_keys,
                read_progress=lambda *_: self.fail("First revision must not read progress"),
            )
        self.assertEqual(calls, ["validate", "preview"])
        self.assertEqual(result["schema"], "work-progress-prepare/v1")
        self.assertEqual(result["progress"]["revision"], 1)


if __name__ == "__main__":
    unittest.main()
