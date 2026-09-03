from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.skills.catalog import SkillRoot, build_skill_catalog, snapshot_catalog_skill


SKILL = """---
name: {name}
description: {description}
metadata:
  work-modes: plan,task,execute
  work-tags: web,frontend
---
{body}
"""


class SkillCatalogTests(unittest.TestCase):
    def _write_skill(self, root: Path, folder: str, *, name: str, body: str = "Instructions") -> Path:
        skill = root / folder
        skill.mkdir(parents=True)
        (skill / "SKILL.md").write_text(
            SKILL.format(name=name, description=f"Use {name} for tests.", body=body),
            encoding="utf-8",
        )
        return skill

    def test_catalog_keeps_same_name_from_different_scopes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            repo = base / "repo"
            user = base / "user"
            repo.mkdir()
            user.mkdir()
            self._write_skill(repo, "repo-ui", name="ui")
            user_skill = self._write_skill(user, "user-ui", name="ui")
            agents = user_skill / "agents"
            agents.mkdir()
            (agents / "openai.yaml").write_text(
                """policy:
  allow_implicit_invocation: false
dependencies:
  tools:
    - type: mcp
      value: designSystem
      description: Design system server
""",
                encoding="utf-8",
            )

            result = build_skill_catalog(
                [
                    SkillRoot("repo", ".agents/skills", repo),
                    SkillRoot("user", ".agents/skills", user),
                ]
            )

        self.assertEqual(len(result["skills"]), 2)
        first, second = result["skills"]
        self.assertNotEqual(first["id"], second["id"])
        self.assertEqual({first["scope"], second["scope"]}, {"repo", "user"})
        explicit = next(skill for skill in result["skills"] if skill["scope"] == "user")
        self.assertFalse(explicit["allow_implicit_invocation"])
        self.assertEqual(explicit["dependencies"][0]["value"], "designSystem")

    def test_catalog_does_not_decode_skill_body(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            skill = root / "binary-body"
            skill.mkdir()
            (skill / "SKILL.md").write_bytes(
                b"---\nname: binary-body\ndescription: Summary only.\n---\n\xff"
            )

            result = build_skill_catalog(
                [SkillRoot("repo", ".agents/skills", root)]
            )

        self.assertEqual([item["name"] for item in result["skills"]], ["binary-body"])
        self.assertEqual(result["unavailable"], [])

    def test_invalid_yaml_is_reported_as_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            skill = root / "invalid"
            skill.mkdir()
            (skill / "SKILL.md").write_text(
                "---\nname: [invalid\ndescription: broken\n---\n",
                encoding="utf-8",
            )

            result = build_skill_catalog(
                [SkillRoot("repo", ".agents/skills", root)]
            )

        self.assertEqual(result["skills"], [])
        self.assertEqual(result["unavailable"][0]["code"], "invalid_skill_frontmatter")

    def test_snapshot_fingerprint_changes_with_bundle_content(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            skill = self._write_skill(root, "frontend", name="frontend")
            references = skill / "references"
            references.mkdir()
            reference = references / "guide.md"
            reference.write_text("Version one\n", encoding="utf-8")
            first = snapshot_catalog_skill(
                SkillRoot("repo", ".agents/skills", root),
                "frontend/SKILL.md",
            )
            reference.write_text("Version two\n", encoding="utf-8")
            second = snapshot_catalog_skill(
                SkillRoot("repo", ".agents/skills", root),
                "frontend/SKILL.md",
            )

        self.assertNotEqual(
            first["bundle"]["bundle_sha256"],
            second["bundle"]["bundle_sha256"],
        )

    def test_root_locator_participates_in_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self._write_skill(root, "ui", name="ui")
            first = build_skill_catalog(
                [SkillRoot("repo", ".agents/skills", root)]
            )["skills"][0]
            second = build_skill_catalog(
                [SkillRoot("repo", "services/api/.agents/skills", root)]
            )["skills"][0]

        self.assertNotEqual(first["id"], second["id"])
        self.assertNotEqual(first["root"], second["root"])


if __name__ == "__main__":
    unittest.main()
