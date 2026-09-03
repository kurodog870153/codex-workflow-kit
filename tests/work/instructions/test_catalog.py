from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.instructions.catalog import (
    build_cross_mode_instruction_catalog,
    build_instruction_catalog,
    resolve_instruction_hierarchy,
)


class InstructionCatalogTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.skill_root = Path(self.temporary_directory.name) / "work"

    def _write_entrypoint(self, mode: str, hierarchy_path: str) -> Path:
        entrypoint = (
            self.skill_root
            / "references"
            / "instructions"
            / mode
            / Path(*hierarchy_path.split("/"))
            / "instructions.md"
        )
        entrypoint.parent.mkdir(parents=True, exist_ok=True)
        entrypoint.write_text(
            "---\n"
            f"name: {hierarchy_path}\n"
            f"description: {hierarchy_path} instructions.\n"
            "metadata:\n"
            "  work-tags:\n"
            "    - test-tag\n"
            "---\n\n"
            f"# {hierarchy_path}\n",
            encoding="utf-8",
        )
        return entrypoint

    def test_catalog_is_sorted_and_reports_immediate_children(self) -> None:
        for hierarchy_path in (
            "web/backend/java/mybatis",
            "general",
            "web/backend/java",
            "web",
            "web/backend/java/jpa",
            "web/backend",
        ):
            self._write_entrypoint("task", hierarchy_path)

        catalog = build_instruction_catalog(self.skill_root, "task")

        self.assertEqual(
            catalog.paths,
            (
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
            ),
        )
        self.assertEqual(catalog.children["general"], ("web",))
        self.assertEqual(catalog.children["web"], ("backend",))
        self.assertEqual(catalog.children["web/backend"], ("java",))
        self.assertEqual(
            catalog.children["web/backend/java"],
            ("jpa", "mybatis"),
        )
        self.assertEqual(catalog.children["web/backend/java/jpa"], ())
        self.assertEqual(
            catalog.metadata["web/backend/java/jpa"],
            {
                "name": "web/backend/java/jpa",
                "description": "web/backend/java/jpa instructions.",
                "work_tags": ["test-tag"],
            },
        )

    def test_catalog_exposes_metadata_without_instruction_body(self) -> None:
        entrypoint = self._write_entrypoint("plan", "general")
        entrypoint.write_text(
            "---\n"
            "name: General\n"
            "description: General planning instructions.\n"
            "metadata:\n"
            "  work-tags:\n"
            "    - requirements-planning\n"
            "---\n\n"
            "# Private instruction body\n",
            encoding="utf-8",
        )

        catalog = build_instruction_catalog(self.skill_root, "plan").as_dict()

        self.assertEqual(
            catalog["metadata"],
            {
                "general": {
                    "name": "General",
                    "description": "General planning instructions.",
                    "work_tags": ["requirements-planning"],
                }
            },
        )
        self.assertNotIn("Private instruction body", str(catalog))
        self.assertRegex(catalog["catalog_sha256"], r"^[0-9a-f]{64}$")

    def test_cross_mode_catalog_unions_paths_and_preserves_mode_metadata(self) -> None:
        for mode in ("plan", "task", "execute"):
            for hierarchy_path in ("general", "web", "web/frontend"):
                self._write_entrypoint(mode, hierarchy_path)
        self._write_entrypoint("plan", "web/frontend/typescript")
        self._write_entrypoint("task", "web/frontend/css")
        self._write_entrypoint("execute", "web/frontend/css")

        catalog = build_cross_mode_instruction_catalog(self.skill_root).as_dict()

        self.assertEqual(catalog["mode"], "all")
        self.assertEqual(
            catalog["paths"],
            [
                "general",
                "web",
                "web/frontend",
                "web/frontend/css",
                "web/frontend/typescript",
            ],
        )
        self.assertEqual(
            catalog["metadata"]["web/frontend/typescript"]["mode_support"],
            ["plan"],
        )
        self.assertEqual(
            set(catalog["metadata"]["web/frontend/css"]["modes"]),
            {"task", "execute"},
        )
        self.assertRegex(catalog["catalog_sha256"], r"^[0-9a-f]{64}$")

    def test_missing_frontmatter_is_rejected(self) -> None:
        entrypoint = self._write_entrypoint("plan", "general")
        entrypoint.write_text("# general\n", encoding="utf-8")

        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "plan")
        self.assertEqual(context.exception.code, "instruction_frontmatter_missing")

    def test_unknown_metadata_field_is_rejected(self) -> None:
        entrypoint = self._write_entrypoint("plan", "general")
        entrypoint.write_text(
            "---\n"
            "name: General\n"
            "description: General planning instructions.\n"
            "metadata:\n"
            "  work-tags:\n"
            "    - requirements-planning\n"
            "  extra: rejected\n"
            "---\n",
            encoding="utf-8",
        )

        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "plan")
        self.assertEqual(
            context.exception.code,
            "invalid_instruction_metadata_fields",
        )

    def test_duplicate_work_tags_are_rejected(self) -> None:
        entrypoint = self._write_entrypoint("plan", "general")
        entrypoint.write_text(
            "---\n"
            "name: General\n"
            "description: General planning instructions.\n"
            "metadata:\n"
            "  work-tags:\n"
            "    - duplicate\n"
            "    - duplicate\n"
            "---\n",
            encoding="utf-8",
        )

        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "plan")
        self.assertEqual(context.exception.code, "invalid_instruction_work_tags")

    def test_directories_without_entrypoint_are_ignored(self) -> None:
        self._write_entrypoint("plan", "general")
        references = (
            self.skill_root
            / "references"
            / "instructions"
            / "plan"
            / "general"
            / "references"
        )
        references.mkdir(parents=True)
        (references / "notes.md").write_text("notes\n", encoding="utf-8")

        catalog = build_instruction_catalog(self.skill_root, "plan")

        self.assertEqual(catalog.paths, ("general",))
        self.assertEqual(catalog.children, {"general": ()})

    def test_invalid_mode_is_rejected(self) -> None:
        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "build")
        self.assertEqual(context.exception.code, "invalid_instruction_mode")

    def test_missing_general_is_rejected(self) -> None:
        self._write_entrypoint("execute", "web")

        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "execute")
        self.assertEqual(context.exception.code, "general_instruction_not_unique")

    def test_missing_ancestor_is_rejected(self) -> None:
        self._write_entrypoint("task", "general")
        self._write_entrypoint("task", "web/backend")

        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "task")
        self.assertEqual(context.exception.code, "instruction_ancestor_missing")
        self.assertEqual(context.exception.details["missing_ancestor"], "web")

    def test_entrypoint_symlink_cannot_escape_skill_root(self) -> None:
        self._write_entrypoint("plan", "general")
        external_entrypoint = Path(self.temporary_directory.name) / "outside.md"
        external_entrypoint.write_text("outside\n", encoding="utf-8")
        escaped_entrypoint = (
            self.skill_root
            / "references"
            / "instructions"
            / "plan"
            / "escaped"
            / "instructions.md"
        )
        escaped_entrypoint.parent.mkdir(parents=True)
        try:
            escaped_entrypoint.symlink_to(external_entrypoint)
        except OSError as error:
            self.skipTest(f"Symbolic links are unavailable: {error}")

        with self.assertRaises(WorkError) as context:
            build_instruction_catalog(self.skill_root, "plan")
        self.assertEqual(
            context.exception.code,
            "instruction_path_escapes_skill_root",
        )

    def test_selected_hierarchy_expands_ordered_leaf_paths(self) -> None:
        for hierarchy_path in (
            "general",
            "web",
            "web/backend",
            "web/backend/java",
            "web/backend/java/jpa",
            "web/backend/java/mybatis",
        ):
            self._write_entrypoint("task", hierarchy_path)

        hierarchy = resolve_instruction_hierarchy(
            self.skill_root,
            "task",
            ["web/backend/java/jpa", "web/backend/java/mybatis"],
        )

        self.assertEqual(
            hierarchy.selected_paths,
            ("web/backend/java/jpa", "web/backend/java/mybatis"),
        )
        self.assertEqual(
            hierarchy.resolved_paths,
            (
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
            ),
        )

    def test_empty_selection_resolves_mandatory_general(self) -> None:
        self._write_entrypoint("plan", "general")

        hierarchy = resolve_instruction_hierarchy(self.skill_root, "plan", [])

        self.assertEqual(hierarchy.selected_paths, ())
        self.assertEqual(hierarchy.resolved_paths, ("general",))
        self.assertEqual(hierarchy.required_paths, ("general",))

    def test_plan_projects_cross_mode_leaf_to_deepest_available_ancestor(self) -> None:
        for mode in ("plan", "task", "execute"):
            for hierarchy_path in ("general", "web", "web/frontend"):
                self._write_entrypoint(mode, hierarchy_path)
        self._write_entrypoint("plan", "web/frontend/typescript")
        for mode in ("task", "execute"):
            self._write_entrypoint(mode, "web/frontend/typescript")
            self._write_entrypoint(mode, "web/frontend/typescript/astro")

        hierarchy = resolve_instruction_hierarchy(
            self.skill_root,
            "plan",
            ["web/frontend/typescript/astro"],
        )

        self.assertEqual(
            hierarchy.selected_paths,
            ("web/frontend/typescript/astro",),
        )
        self.assertEqual(
            hierarchy.resolved_paths,
            ("general", "web", "web/frontend", "web/frontend/typescript"),
        )

    def test_explicit_general_is_rejected(self) -> None:
        self._write_entrypoint("plan", "general")

        with self.assertRaises(WorkError) as context:
            resolve_instruction_hierarchy(self.skill_root, "plan", ["general"])
        self.assertEqual(context.exception.code, "invalid_hierarchy_path")

    def test_redundant_ancestor_selection_is_rejected(self) -> None:
        for hierarchy_path in ("general", "web", "web/backend"):
            self._write_entrypoint("task", hierarchy_path)

        with self.assertRaises(WorkError) as context:
            resolve_instruction_hierarchy(
                self.skill_root,
                "task",
                ["web", "web/backend"],
            )
        self.assertEqual(context.exception.code, "redundant_hierarchy_path")

    def test_missing_selected_path_reports_valid_immediate_choices(self) -> None:
        for hierarchy_path in (
            "general",
            "web",
            "web/backend",
            "web/backend/java",
            "web/backend/java/jpa",
            "web/backend/java/mybatis",
        ):
            self._write_entrypoint("execute", hierarchy_path)

        with self.assertRaises(WorkError) as context:
            resolve_instruction_hierarchy(
                self.skill_root,
                "execute",
                ["web/backend/java/hibernate"],
            )

        self.assertEqual(
            context.exception.code,
            "instruction_hierarchy_path_missing",
        )
        self.assertEqual(
            context.exception.details,
            {
                "mode": "execute",
                "path": "web/backend/java/hibernate",
                "parent": "web/backend/java",
                "valid_choices": ["jpa", "mybatis"],
            },
        )


if __name__ == "__main__":
    unittest.main()
