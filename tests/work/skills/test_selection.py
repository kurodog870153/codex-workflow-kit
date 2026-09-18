from __future__ import annotations

import sys
import copy
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.services.skill_catalog import SkillRoot, snapshot_catalog_skill
from worklib.services.skill_selection import build_skill_selection, selection_sha256, validate_skill_selection


class SkillSelectionTests(unittest.TestCase):
    def _snapshots(self, *roots):
        return {(root.scope, root.locator, "frontend/SKILL.md"): snapshot_catalog_skill(root, "frontend/SKILL.md") for root in roots}

    def _choice(self, selected):
        return {key: selected[key] for key in (
            "scope", "root", "source", "recommendation_reason", "dependency_status",
        )}

    def test_build_declared_selection_matches_validated_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            request = {"decision": "external_skills", "skills": [self._choice(selected)]}
            before = copy.deepcopy(request)
            result = build_skill_selection(request, roots=[root], snapshots=self._snapshots(root))
            self.assertEqual(result, self._selection([selected]))
            self.assertEqual(request, before)

    def test_build_preserves_order_and_distinguishes_equal_names_across_roots(self):
        with tempfile.TemporaryDirectory() as temporary:
            first, first_selected = self._fixture(Path(temporary) / "first")
            second, _ = self._fixture(Path(temporary) / "second")
            second = SkillRoot("user", "personal", second.path)
            choice = self._choice(first_selected)
            other = {**choice, "scope": "user", "root": "personal"}
            request = {"decision": "external_skills", "skills": [other, choice]}
            result = build_skill_selection(request, roots=[first, second], snapshots=self._snapshots(first, second))
            self.assertEqual([item["scope"] for item in result["skills"]], ["user", "repo"])
            self.assertEqual(len({item["id"] for item in result["skills"]}), 2)
            reverse = build_skill_selection({**request, "skills": [choice, other]}, roots=[first, second], snapshots=self._snapshots(first, second))
            self.assertNotEqual(result["selection_sha256"], reverse["selection_sha256"])

    def test_build_requires_explicit_base_only_and_valid_request(self):
        self.assertEqual(build_skill_selection({"decision": "base_only", "skills": []}, roots=[], snapshots={}),
                         self._selection([], "base_only"))
        for request in ({"skills": []}, {"decision": "external_skills", "skills": []},
                        {"decision": [], "skills": []}, {"decision": "base_only", "skills": {}},
                        {"decision": "base_only", "skills": [], "selection_sha256": "invented"}):
            with self.subTest(request=request), self.assertRaises(WorkError):
                build_skill_selection(request, roots=[], snapshots={})

    def test_build_rejects_duplicates_missing_roots_and_unavailable_dependencies(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            choice = self._choice(selected)
            for choices, roots, code in (
                ([choice, choice], [root], "duplicate_selected_skill"),
                ([choice], [], "selected_skill_root_missing"),
                ([{**choice, "dependency_status": "unavailable"}], [root], "skill_dependencies_unavailable"),
            ):
                with self.subTest(code=code), self.assertRaises(WorkError) as caught:
                    build_skill_selection({"decision": "external_skills", "skills": choices}, roots=roots, snapshots=self._snapshots(*roots))
                self.assertEqual(caught.exception.code, code)

    def test_build_requires_inferred_modes_and_rejects_declared_mode_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            choice = self._choice(selected)
            inferred = {"plan": "inferred", "task": "inferred", "execute": "unsupported"}
            with self.assertRaises(WorkError) as caught:
                build_skill_selection({"decision": "external_skills", "skills": [{**choice, "mode_support": inferred}]}, roots=[root], snapshots=self._snapshots(root))
            self.assertEqual(caught.exception.code, "declared_skill_modes_mismatch")
            path = root.path / "frontend/SKILL.md"
            path.write_text("---\nname: frontend\ndescription: Build frontends.\n---\nInstructions\n")
            with self.assertRaises(WorkError) as caught:
                build_skill_selection({"decision": "external_skills", "skills": [choice]}, roots=[root], snapshots=self._snapshots(root))
            self.assertEqual(caught.exception.code, "inferred_skill_modes_required")
            result = build_skill_selection({"decision": "external_skills", "skills": [{**choice, "mode_support": inferred}]}, roots=[root], snapshots=self._snapshots(root))
            self.assertEqual(result["skills"][0]["mode_support"], inferred)

    def test_build_rejects_stale_catalog_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            path = root.path / "frontend/SKILL.md"

            snapshots = self._snapshots(root)
            path.write_text(path.read_text() + "Changed\n")
            fresh = self._snapshots(root)
            built = build_skill_selection({"decision": "external_skills", "skills": [self._choice(selected)]}, roots=[root], snapshots=snapshots)
            with self.assertRaises(WorkError) as caught:
                validate_skill_selection(built, roots=[root], snapshots=fresh)
            self.assertEqual(caught.exception.code, "selected_skill_snapshot_mismatch")

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
                roots=[root], snapshots=self._snapshots(root),
            )

        self.assertEqual(result["status"], "valid")

    def test_base_only_requires_empty_skills(self) -> None:
        result = validate_skill_selection(
            self._selection([], decision="base_only"),
            roots=[], snapshots={},
        )
        self.assertEqual(result["status"], "valid")

        with self.assertRaises(WorkError) as context:
            validate_skill_selection(
                self._selection([], decision="external_skills"),
                roots=[], snapshots={},
            )
        self.assertEqual(context.exception.code, "skill_selection_decision_mismatch")

    def test_duplicate_skill_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root, selected = self._fixture(Path(temporary))
            selection = self._selection([selected, dict(selected)])
            with self.assertRaises(WorkError) as context:
                validate_skill_selection(selection, roots=[root], snapshots=self._snapshots(root))

        self.assertEqual(context.exception.code, "duplicate_selected_skill")

    def test_bundle_drift_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root, selected = self._fixture(base)
            selection = self._selection([selected])
            skill_file = root.path / "frontend" / "SKILL.md"
            skill_file.write_text(skill_file.read_text(encoding="utf-8") + "Changed\n", encoding="utf-8")
            with self.assertRaises(WorkError) as context:
                validate_skill_selection(selection, roots=[root], snapshots=self._snapshots(root))

        self.assertEqual(context.exception.code, "selected_skill_snapshot_mismatch")


if __name__ == "__main__":
    unittest.main()
