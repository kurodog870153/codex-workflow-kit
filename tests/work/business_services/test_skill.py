from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.business_services.skill import build_selection, bundle
from worklib.foundation.errors import WorkError
from worklib.models.skill import SkillRoot
from worklib.services.skill_catalog import snapshot_catalog_skill


class SkillBusinessServiceTests(unittest.TestCase):
    def test_bundle_preserves_the_compatibility_entrypoint(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            skill_root = Path(temporary) / "frontend"
            skill_root.mkdir()
            (skill_root / "SKILL.md").write_text("---\nname: frontend\ndescription: Build frontends.\n---\nInstructions\n", encoding="utf-8")
            result = bundle(skill_root)
        self.assertEqual(result["schema"], "work-skill-bundle/v1")
        self.assertEqual(len(result["bundle_sha256"]), 64)

    def test_build_resnapshots_to_reject_source_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root_path = Path(temporary) / "skills"
            skill_path = root_path / "frontend"
            skill_path.mkdir(parents=True)
            skill_file = skill_path / "SKILL.md"
            skill_file.write_text("---\nname: frontend\ndescription: Build frontends.\nmetadata:\n  work-modes: plan\n---\nInstructions\n", encoding="utf-8")
            root = SkillRoot("repo", ".agents/skills", root_path)
            selected = snapshot_catalog_skill(root, "frontend/SKILL.md")["skill"]
            choice = {"scope": selected["scope"], "root": selected["root"], "source": selected["source"],
                      "recommendation_reason": "Needed.", "dependency_status": "available"}
            original = snapshot_catalog_skill
            calls = 0

            def drifting(*args, **kwargs):
                nonlocal calls
                result = original(*args, **kwargs)
                calls += 1
                if calls == 1:
                    skill_file.write_text(skill_file.read_text(encoding="utf-8") + "Changed\n", encoding="utf-8")
                return result

            with patch("worklib.business_services.skill.snapshot_catalog_skill", side_effect=drifting):
                with self.assertRaises(WorkError) as caught:
                    build_selection(json.dumps({"decision": "external_skills", "skills": [choice]}).encode(),
                                    source="request.json", roots=[root])
        self.assertEqual(caught.exception.code, "selected_skill_snapshot_mismatch")


if __name__ == "__main__":
    unittest.main()
