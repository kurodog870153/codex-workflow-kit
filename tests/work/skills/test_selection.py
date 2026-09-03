from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.skills.catalog import SkillRoot, snapshot_catalog_skill
from worklib.skills.selection import selection_sha256, validate_skill_selection


class SkillSelectionTests(unittest.TestCase):
    def _fixture(self, base: Path) -> tuple[SkillRoot, dict[str, object]]:
        root_path = base / "skills"
        skill = root_path / "frontend"
        skill.mkdir(parents=True)
        (skill / "SKILL.md").write_text(
            """---
name: frontend
description: Build frontends.
metadata:
  work-modes: plan,task,execute
---
Instructions
""",
            encoding="utf-8",
        )
        root = SkillRoot("repo", ".agents/skills", root_path)
        snapshot = snapshot_catalog_skill(root, "frontend/SKILL.md")
        summary = snapshot["skill"]
        bundle = snapshot["bundle"]
        assert isinstance(summary, dict)
        assert isinstance(bundle, dict)
        selected = {
            "id": summary["id"],
            "name": summary["name"],
            "scope": summary["scope"],
            "root": summary["root"],
            "source": summary["source"],
            "description": summary["description"],
            "mode_support": {
                "plan": "declared",
                "task": "declared",
                "execute": "declared",
            },
            "allow_implicit_invocation": summary["allow_implicit_invocation"],
            "dependency_status": "available",
            "summary_sha256": summary["summary_sha256"],
            "bundle_sha256": bundle["bundle_sha256"],
            "recommendation_reason": "The request requires a frontend.",
        }
        return root, selected

    def _selection(self, skills: list[dict[str, object]], decision: str = "external_skills") -> dict[str, object]:
        return {
            "schema": "work-skill-selection/v1",
            "decision": decision,
            "skills": skills,
            "selection_sha256": selection_sha256(decision, skills),
        }

    def test_valid_external_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            result = validate_skill_selection(
                self._selection([selected]),
                roots=[root],
            )

        self.assertEqual(result["status"], "valid")

    def test_base_only_requires_empty_skills(self) -> None:
        result = validate_skill_selection(
            self._selection([], decision="base_only"),
            roots=[],
        )
        self.assertEqual(result["status"], "valid")

        with self.assertRaises(WorkError) as context:
            validate_skill_selection(
                self._selection([], decision="external_skills"),
                roots=[],
            )
        self.assertEqual(context.exception.code, "skill_selection_decision_mismatch")

    def test_duplicate_skill_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            selection = self._selection([selected, dict(selected)])
            with self.assertRaises(WorkError) as context:
                validate_skill_selection(selection, roots=[root])

        self.assertEqual(context.exception.code, "duplicate_selected_skill")

    def test_bundle_drift_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root, selected = self._fixture(base)
            selection = self._selection([selected])
            skill_file = root.path / "frontend" / "SKILL.md"
            skill_file.write_text(skill_file.read_text(encoding="utf-8") + "Changed\n", encoding="utf-8")
            with self.assertRaises(WorkError) as context:
                validate_skill_selection(selection, roots=[root])

        self.assertEqual(context.exception.code, "selected_skill_snapshot_mismatch")


if __name__ == "__main__":
    unittest.main()
