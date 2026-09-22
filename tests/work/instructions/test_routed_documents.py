from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "references"


class RoutedDocumentTests(unittest.TestCase):
    def test_bootstrap_is_small_and_forbids_full_fallback(self) -> None:
        text = (ROOT / "instruction-loading.md").read_text(encoding="utf-8")
        self.assertLessEqual(len(text.splitlines()), 15)
        self.assertIn("Do not guess sources", text)
        self.assertIn("Never fall back", text)

    def test_common_modules_are_single_topic_and_do_not_link_to_modules(self) -> None:
        modules = sorted((ROOT / "instruction-loading").glob("*.md"))
        self.assertEqual(len(modules), 18)
        for path in modules:
            text = path.read_text(encoding="utf-8")
            self.assertEqual(len(re.findall(r"^# ", text, re.MULTILINE)), 1)
            self.assertNotRegex(text, r"\]\([^)]*\.md")

    def test_workflow_entries_are_small_and_operation_modules_do_not_cross_link(self) -> None:
        for entry in sorted((ROOT / "workflows").glob("*.md")):
            self.assertLessEqual(len(entry.read_text(encoding="utf-8").splitlines()), 15)
            modules = sorted(entry.with_suffix("").glob("*.md"))
            self.assertTrue(modules, entry.name)
            for module in modules:
                self.assertNotRegex(
                    module.read_text(encoding="utf-8"), r"\]\([^)]*\.md",
                )


if __name__ == "__main__":
    unittest.main()
