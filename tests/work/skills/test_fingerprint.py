from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.skills.fingerprint import snapshot_skill_bundle


class SkillFingerprintTests(unittest.TestCase):
    def test_bundle_fingerprint_changes_with_reference_content(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            skill = Path(temporary) / "frontend"
            references = skill / "references"
            references.mkdir(parents=True)
            (skill / "SKILL.md").write_text(
                "---\nname: frontend\ndescription: Frontend skill.\n---\n",
                encoding="utf-8",
            )
            reference = references / "guide.md"
            reference.write_text("Version one\n", encoding="utf-8")
            first = snapshot_skill_bundle(skill)

            reference.write_text("Version two\n", encoding="utf-8")
            second = snapshot_skill_bundle(skill)

        self.assertNotEqual(
            first["bundle_sha256"],
            second["bundle_sha256"],
        )


if __name__ == "__main__":
    unittest.main()
