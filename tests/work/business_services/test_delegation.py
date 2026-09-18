from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_delegation as fixtures
from contracts import test_task as task_fixtures
from worklib.business_services.delegation import validate_delegation


class DelegationBusinessServiceTests(unittest.TestCase):
    def test_public_entry_validates_complete_context(self) -> None:
        fixture = fixtures.DelegationTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        value = fixture.envelope()
        result = validate_delegation(
            value,
            role="plan",
            sender="parent",
            project_root=fixture.root,
            skill_root=task_fixtures.SKILL_ROOT,
        )
        self.assertEqual(result["status"], "valid")


if __name__ == "__main__":
    unittest.main()
