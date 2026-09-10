from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.hierarchy.selection import (
    build_hierarchy_selection,
    validate_hierarchy_selection,
    validate_task_hierarchy_paths,
)


class HierarchySelectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.skill_root = Path(self.temporary_directory.name) / "work"
        for mode in ("plan", "task", "execute"):
            paths = ["general", "web", "web/frontend", "web/frontend/typescript"]
            if mode != "plan":
                paths.append("web/frontend/typescript/astro")
            for hierarchy_path in paths:
                self._write_entrypoint(mode, hierarchy_path)

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
            f"name: {mode} {hierarchy_path}\n"
            f"description: Use {hierarchy_path} during {mode}.\n"
            "metadata:\n"
            "  work-tags:\n"
            f"    - {mode}-test\n"
            "---\n\n"
            f"# {hierarchy_path}\n",
            encoding="utf-8",
        )
        return entrypoint

    def _request(self) -> dict[str, object]:
        return {
            "decision": "instruction_paths",
            "selections": [
                {
                    "path": "web/frontend/typescript/astro",
                    "recommendation_reason": "The requested implementation uses Astro.",
                }
            ],
        }

    def test_build_snapshots_cross_mode_leaf_metadata_and_hashes(self) -> None:
        selection = build_hierarchy_selection(
            self._request(),
            skill_root=self.skill_root,
        )

        self.assertEqual(
            selection["selected_paths"],
            ["web/frontend/typescript/astro"],
        )
        entry = selection["entries"][0]
        self.assertEqual(entry["mode_support"], ["task", "execute"])
        self.assertEqual(set(entry["mode_metadata"]), {"task", "execute"})
        self.assertRegex(selection["catalog_sha256"], r"^[0-9a-f]{64}$")
        self.assertRegex(selection["selection_sha256"], r"^[0-9a-f]{64}$")

    def test_general_only_is_explicit_and_has_no_entries(self) -> None:
        selection = build_hierarchy_selection(
            {"decision": "general_only", "selections": []},
            skill_root=self.skill_root,
        )

        self.assertEqual(selection["selected_paths"], [])
        self.assertEqual(selection["entries"], [])
        validated = validate_hierarchy_selection(
            selection,
            skill_root=self.skill_root,
        )
        self.assertEqual(validated["status"], "valid")

    def test_intermediate_path_is_accepted_and_validated(self) -> None:
        request = self._request()
        request["selections"][0]["path"] = "web/frontend/typescript"

        selection = build_hierarchy_selection(request, skill_root=self.skill_root)
        validated = validate_hierarchy_selection(selection, skill_root=self.skill_root)

        self.assertEqual(selection["selected_paths"], ["web/frontend/typescript"])
        self.assertEqual(validated["status"], "valid")

    def test_java_plan_authorizes_jpa_and_mybatis_tasks(self) -> None:
        skill_root = SCRIPT_ROOT.parent
        request = self._request()
        request["selections"][0]["path"] = "web/backend/java"
        selection = build_hierarchy_selection(request, skill_root=skill_root)
        self.assertEqual(
            validate_hierarchy_selection(selection, skill_root=skill_root)["status"],
            "valid",
        )
        for path in ("web/backend/java", "web/backend", "web/backend/java/jpa", "web/backend/java/mybatis"):
            with self.subTest(path=path):
                self.assertEqual(
                    validate_task_hierarchy_paths(
                        [path], confirmed_selection=selection,
                        skill_root=skill_root, location="TASK-001",
                    ),
                    (path,),
                )

    def test_descendant_must_exist_in_task_and_execute_catalogs(self) -> None:
        request = self._request()
        request["selections"][0]["path"] = "web/frontend/typescript"
        confirmed = build_hierarchy_selection(request, skill_root=self.skill_root)
        for mode in ("task", "execute"):
            path = f"web/frontend/typescript/{mode}-only"
            self._write_entrypoint(mode, path)
            with self.subTest(mode=mode), self.assertRaises(WorkError) as context:
                validate_task_hierarchy_paths(
                    [path], confirmed_selection=confirmed,
                    skill_root=self.skill_root, location="TASK-001",
                )
            self.assertEqual(context.exception.code, "instruction_hierarchy_path_missing")

    def test_intermediate_selection_does_not_authorize_prefix_sibling(self) -> None:
        for mode in ("task", "execute"):
            self._write_entrypoint(mode, "web/frontend/typescript-extra")
        request = self._request()
        request["selections"][0]["path"] = "web/frontend/typescript"
        confirmed = build_hierarchy_selection(request, skill_root=self.skill_root)
        with self.assertRaises(WorkError) as context:
            validate_task_hierarchy_paths(
                ["web/frontend/typescript-extra"], confirmed_selection=confirmed,
                skill_root=self.skill_root, location="TASK-001",
            )
        self.assertEqual(context.exception.code, "task_hierarchy_path_not_authorized")

    def test_catalog_drift_requires_return_to_plan(self) -> None:
        selection = build_hierarchy_selection(
            self._request(),
            skill_root=self.skill_root,
        )
        entrypoint = (
            self.skill_root
            / "references"
            / "instructions"
            / "task"
            / "web"
            / "frontend"
            / "typescript"
            / "astro"
            / "instructions.md"
        )
        text = entrypoint.read_text(encoding="utf-8")
        entrypoint.write_text(
            text.replace("during task.", "for task implementation."),
            encoding="utf-8",
        )

        with self.assertRaises(WorkError) as context:
            validate_hierarchy_selection(selection, skill_root=self.skill_root)

        self.assertEqual(
            context.exception.code,
            "instruction_catalog_snapshot_mismatch",
        )

    def test_tampered_selection_hash_is_rejected(self) -> None:
        selection = build_hierarchy_selection(
            self._request(),
            skill_root=self.skill_root,
        )
        tampered = copy.deepcopy(selection)
        tampered["selection_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            validate_hierarchy_selection(tampered, skill_root=self.skill_root)

        self.assertEqual(
            context.exception.code,
            "hierarchy_selection_fingerprint_mismatch",
        )

    def test_task_path_may_use_confirmed_leaf_or_ancestor(self) -> None:
        confirmed = build_hierarchy_selection(
            self._request(),
            skill_root=self.skill_root,
        )

        selected = validate_task_hierarchy_paths(
            ["web/frontend/typescript"],
            confirmed_selection=confirmed,
            skill_root=self.skill_root,
            location="TASK-001.instruction_selection.selected_paths",
        )

        self.assertEqual(selected, ("web/frontend/typescript",))

    def test_task_path_outside_confirmed_branch_is_rejected(self) -> None:
        for mode in ("plan", "task", "execute"):
            self._write_entrypoint(mode, "web/frontend/css")
        confirmed = build_hierarchy_selection(
            self._request(),
            skill_root=self.skill_root,
        )

        with self.assertRaises(WorkError) as context:
            validate_task_hierarchy_paths(
                ["web/frontend/css"],
                confirmed_selection=confirmed,
                skill_root=self.skill_root,
                location="TASK-001.instruction_selection.selected_paths",
            )

        self.assertEqual(context.exception.code, "task_hierarchy_path_not_authorized")

    def test_installed_catalog_selects_astro_and_tailwind_leaves(self) -> None:
        project_root = Path(__file__).resolve().parents[3]
        installed_skill_root = project_root / "skills" / "work"

        selection = build_hierarchy_selection(
            {
                "decision": "instruction_paths",
                "selections": [
                    {
                        "path": "web/frontend/typescript/astro",
                        "recommendation_reason": "The implementation uses Astro.",
                    },
                    {
                        "path": "web/frontend/css/tailwind",
                        "recommendation_reason": "The implementation uses Tailwind.",
                    },
                ],
            },
            skill_root=installed_skill_root,
        )

        self.assertEqual(
            selection["selected_paths"],
            [
                "web/frontend/typescript/astro",
                "web/frontend/css/tailwind",
            ],
        )
        self.assertEqual(selection["entries"][0]["mode_support"], ["task", "execute"])
        self.assertEqual(selection["entries"][1]["mode_support"], ["task", "execute"])


if __name__ == "__main__":
    unittest.main()
