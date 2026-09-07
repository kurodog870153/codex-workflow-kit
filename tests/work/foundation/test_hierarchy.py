from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.foundation.hierarchy import build_hierarchy


class HierarchyTests(unittest.TestCase):
    def test_build_expands_ancestors_and_classifies_paths(self) -> None:
        hierarchy = build_hierarchy(
            "task",
            ["web/backend/java", "web/frontend/react"],
        )

        self.assertEqual(
            hierarchy.resolved_paths,
            (
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/frontend",
                "web/frontend/react",
            ),
        )
        self.assertEqual(
            hierarchy.required_paths,
            ("general", "web/backend/java", "web/frontend/react"),
        )
        self.assertEqual(
            hierarchy.optional_paths,
            ("web", "web/backend", "web/frontend"),
        )
        self.assertEqual(
            hierarchy.as_dict(),
            {
                "schema": "work-hierarchy/v1",
                "work_directory": "task",
                "selected_paths": ["web/backend/java", "web/frontend/react"],
                "resolved_paths": [
                    "general",
                    "web",
                    "web/backend",
                    "web/backend/java",
                    "web/frontend",
                    "web/frontend/react",
                ],
                "required_paths": [
                    "general",
                    "web/backend/java",
                    "web/frontend/react",
                ],
                "optional_paths": ["web", "web/backend", "web/frontend"],
            },
        )

    def test_rejects_invalid_work_directory(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_hierarchy("invalid", [])

        self.assertEqual(context.exception.code, "invalid_work_directory")

    def test_rejects_invalid_selected_paths(self) -> None:
        for path in ("", "general", "Web/backend", "web//backend"):
            with self.subTest(path=path), self.assertRaises(WorkError) as context:
                build_hierarchy("execute", [path])

            self.assertEqual(context.exception.code, "invalid_hierarchy_path")

    def test_rejects_duplicate_and_redundant_selected_paths(self) -> None:
        cases = (
            (["web", "web"], "duplicate_hierarchy_path"),
            (["web", "web/backend"], "redundant_hierarchy_path"),
        )
        for selected_paths, expected_code in cases:
            with self.subTest(selected_paths=selected_paths):
                with self.assertRaises(WorkError) as context:
                    build_hierarchy("plan", selected_paths)

                self.assertEqual(context.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()
