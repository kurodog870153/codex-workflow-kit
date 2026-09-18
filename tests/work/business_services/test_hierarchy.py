from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.business_services.hierarchy import build_hierarchy_selection, validate_hierarchy_selection


class HierarchyBusinessServiceTests(unittest.TestCase):
    def test_general_only_round_trip(self) -> None:
        skill_root = SCRIPT_ROOT.parent
        selection = build_hierarchy_selection(
            {"decision": "general_only", "selections": []},
            skill_root=skill_root,
        )
        validation = validate_hierarchy_selection(selection, skill_root=skill_root)
        self.assertEqual(validation["status"], "valid")
        self.assertEqual(validation["hierarchy_selection"], selection)


if __name__ == "__main__":
    unittest.main()
