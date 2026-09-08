from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.foundation.paths import (
    default_artifact_paths,
    normalize_relative_path,
    portable_path_identity,
    resolve_project_relative_path,
    resolve_root,
    validate_artifact_paths,
    validate_requirement_id,
)


class PathTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.project_root = Path(self.temporary_directory.name).resolve()

    def test_resolve_root_requires_directory(self) -> None:
        self.assertEqual(
            resolve_root(str(self.project_root), label="project root"),
            self.project_root,
        )
        file_path = self.project_root / "file.txt"
        file_path.write_text("content", encoding="utf-8")

        with self.assertRaises(WorkError) as context:
            resolve_root(str(file_path), label="project root")

        self.assertEqual(context.exception.code, "root_not_directory")

    def test_requirement_id_rejects_invalid_and_reserved_names(self) -> None:
        self.assertEqual(validate_requirement_id("feature-1.2"), "feature-1.2")
        cases = (
            ("Feature", "invalid_requirement_id"),
            ("con", "windows_device_name"),
        )
        for value, expected_code in cases:
            with self.subTest(value=value):
                with self.assertRaises(WorkError) as context:
                    validate_requirement_id(value)

                self.assertEqual(context.exception.code, expected_code)

    def test_normalizes_relative_separators_and_rejects_unsafe_paths(self) -> None:
        self.assertEqual(
            normalize_relative_path(r".\outputs\work\task.json"),
            "outputs/work/task.json",
        )
        cases = (
            ("../task.json", "unsafe_path_segment"),
            ("C:/task.json", "absolute_path_rejected"),
            ("outputs/NUL.txt", "windows_device_name"),
        )
        for value, expected_code in cases:
            with self.subTest(value=value):
                with self.assertRaises(WorkError) as context:
                    normalize_relative_path(value)

                self.assertEqual(context.exception.code, expected_code)

    def test_resolves_nonexistent_project_relative_path(self) -> None:
        normalized, resolved = resolve_project_relative_path(
            self.project_root,
            "outputs/work/task.json",
        )

        self.assertEqual(normalized, "outputs/work/task.json")
        self.assertEqual(
            resolved,
            self.project_root / "outputs" / "work" / "task.json",
        )

    def test_portable_identity_normalizes_case_and_unicode(self) -> None:
        composed = self.project_root / "CAFÉ.json"
        decomposed = self.project_root / "cafe\N{COMBINING ACUTE ACCENT}.json"

        self.assertEqual(
            portable_path_identity(composed),
            portable_path_identity(decomposed),
        )

    def test_default_and_validated_artifact_paths_match(self) -> None:
        artifacts = default_artifact_paths(self.project_root, "feature-1")

        self.assertEqual(artifacts["task"], "outputs/work/tasks/feature-1/task.json")
        self.assertEqual(
            validate_artifact_paths(
                self.project_root,
                "feature-1",
                artifacts,
                actual_plan_path=artifacts["plan"],
            ),
            artifacts,
        )

    def test_rejects_old_task_layout_and_wrong_requirement_directory(self) -> None:
        artifacts = default_artifact_paths(self.project_root, "feature-1")
        for path in (
            "outputs/work/tasks/feature-1/task.md",
            "outputs/work/tasks/feature-1.json",
            "custom/feature-1.json",
            "outputs/work/tasks/other/task.json",
            "outputs/work/tasks/feature-1/other.json",
            "outputs/work/tasks/feature-1/drafts/task.json",
        ):
            with self.subTest(path=path):
                with self.assertRaises(WorkError) as context:
                    validate_artifact_paths(
                        self.project_root,
                        "feature-1",
                        {**artifacts, "task": path},
                        actual_plan_path=artifacts["plan"],
                    )
                self.assertEqual(context.exception.code, "task_path_requirement_mismatch")

    def test_rejects_legacy_plan_extension(self) -> None:
        artifacts = default_artifact_paths(self.project_root, "feature-1")
        artifacts["plan"] = "outputs/work/plans/feature-1.md"
        with self.assertRaises(WorkError) as context:
            validate_artifact_paths(
                self.project_root, "feature-1", artifacts,
                actual_plan_path=artifacts["plan"],
            )
        self.assertEqual(context.exception.code, "plan_path_requirement_mismatch")

    def test_nondefault_task_route_requires_new_layout(self) -> None:
        artifacts = default_artifact_paths(self.project_root, "feature-1")
        artifacts["task"] = "custom/feature-1/task.json"
        self.assertEqual(
            validate_artifact_paths(
                self.project_root,
                "feature-1",
                artifacts,
                actual_plan_path=artifacts["plan"],
            ),
            artifacts,
        )

    def test_rejects_artifact_path_aliases(self) -> None:
        artifacts = {
            "plan": "outputs/task/task.json",
            "task": "outputs/task/task.json",
            "execution": "outputs/executions/task",
        }

        with self.assertRaises(WorkError) as context:
            validate_artifact_paths(
                self.project_root,
                "task",
                artifacts,
                actual_plan_path=artifacts["plan"],
            )

        self.assertEqual(context.exception.code, "artifact_path_alias")


if __name__ == "__main__":
    unittest.main()
