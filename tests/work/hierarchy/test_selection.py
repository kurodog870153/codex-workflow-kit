from __future__ import annotations

import copy
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import main
from worklib.foundation.errors import ExitCode, WorkError
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

    def test_non_leaf_path_is_rejected(self) -> None:
        request = self._request()
        request["selections"][0]["path"] = "web/frontend/typescript"

        with self.assertRaises(WorkError) as context:
            build_hierarchy_selection(request, skill_root=self.skill_root)

        self.assertEqual(context.exception.code, "hierarchy_selection_path_not_leaf")

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

    def test_cli_build_and_validate_round_trip(self) -> None:
        project_root = str(Path(__file__).resolve().parents[3])
        stdout = io.StringIO()
        stderr = io.StringIO()
        request = {
            "decision": "instruction_paths",
            "selections": [
                {
                    "path": "web/backend/java/jpa",
                    "recommendation_reason": "The work uses JPA persistence.",
                }
            ],
        }

        exit_code = main(
            [
                "--project-root",
                project_root,
                "hierarchy",
                "selection-build",
                "--stdin",
            ],
            stdin=io.StringIO(json.dumps(request)),
            stdout=stdout,
            stderr=stderr,
        )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        selection = json.loads(stdout.getvalue())
        validate_stdout = io.StringIO()
        validate_stderr = io.StringIO()
        validate_exit_code = main(
            [
                "--project-root",
                project_root,
                "hierarchy",
                "selection-validate",
                "--stdin",
            ],
            stdin=io.StringIO(json.dumps(selection)),
            stdout=validate_stdout,
            stderr=validate_stderr,
        )

        self.assertEqual(validate_exit_code, ExitCode.SUCCESS)
        self.assertEqual(validate_stderr.getvalue(), "")
        self.assertEqual(json.loads(validate_stdout.getvalue())["status"], "valid")

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
