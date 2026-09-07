from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.hierarchy.ordering import order_hierarchy_selection


class HierarchyOrderingTests(unittest.TestCase):
    def test_orders_selection_entries_modes_and_metadata(self) -> None:
        ordered = order_hierarchy_selection(
            {
                "zzz": 2,
                "selection_sha256": "a" * 64,
                "catalog_sha256": "b" * 64,
                "entries": [
                    {
                        "extra": True,
                        "recommendation_reason": "Selected.",
                        "mode_metadata": {
                            "zzz": {"name": "Unknown"},
                            "execute": {
                                "work_tags": ["execute"],
                                "description": "Execute metadata.",
                                "name": "Execute",
                            },
                            "plan": {
                                "extra": True,
                                "work_tags": ["plan"],
                                "name": "Plan",
                            },
                        },
                        "mode_support": ["plan", "execute"],
                        "path": "web/backend",
                    }
                ],
                "selected_paths": ["web/backend"],
                "decision": "instruction_paths",
                "schema": "work-hierarchy-selection/v1",
                "aaa": 1,
            }
        )

        self.assertEqual(
            list(ordered),
            [
                "schema",
                "decision",
                "selected_paths",
                "entries",
                "catalog_sha256",
                "selection_sha256",
                "aaa",
                "zzz",
            ],
        )
        entry = ordered["entries"][0]
        self.assertEqual(
            list(entry),
            [
                "path",
                "mode_support",
                "mode_metadata",
                "recommendation_reason",
                "extra",
            ],
        )
        self.assertEqual(
            list(entry["mode_metadata"]),
            ["plan", "execute", "zzz"],
        )
        self.assertEqual(
            list(entry["mode_metadata"]["plan"]),
            ["name", "work_tags", "extra"],
        )
        self.assertEqual(
            list(entry["mode_metadata"]["execute"]),
            ["name", "description", "work_tags"],
        )

    def test_nonobject_value_is_unchanged(self) -> None:
        value = ["general"]

        self.assertIs(order_hierarchy_selection(value), value)


if __name__ == "__main__":
    unittest.main()
