from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.work_paths import (
    default_artifact_paths,
    default_task_collection_artifact_paths,
    normalize_relative_path,
    portable_path_identity,
    resolve_project_relative_path,
    resolve_task_collection_item_path,
    resolve_root,
    validate_artifact_paths,
    validate_task_collection_index_path,
    validate_task_item_path_aliases,
)
from worklib.models.common.identifiers import IdentifierPolicy
from worklib.services.specification.transaction import transaction_directory

validate_requirement_id = IdentifierPolicy.requirement_id
validate_transaction_id = IdentifierPolicy.transaction_id
validate_workflow_id = IdentifierPolicy.workflow_id


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

    def test_transaction_directory_uses_requirement_or_pending_owner(self) -> None:
        transaction_id = "20260915T103000Z-a1b2c3d4"
        normalized, resolved = transaction_directory(
            self.project_root,
            requirement_id="feature-1",
            workflow_id="specification",
            transaction_id=transaction_id,
        )
        self.assertEqual(
            normalized,
            f"outputs/work/transactions/feature-1/specification/{transaction_id}",
        )
        self.assertEqual(resolved, self.project_root.joinpath(*normalized.split("/")))

        pending, _ = transaction_directory(
            self.project_root,
            requirement_id=None,
            workflow_id="invocation",
            transaction_id=transaction_id,
        )
        self.assertEqual(
            pending,
            f"outputs/work/transactions/pending/invocation/{transaction_id}",
        )

    def test_transaction_segments_reject_unsafe_or_ambiguous_values(self) -> None:
        self.assertEqual(validate_workflow_id("specification-update"), "specification-update")
        self.assertEqual(
            validate_transaction_id("20260915T103000Z-a1b2c3d4"),
            "20260915T103000Z-a1b2c3d4",
        )
        cases = (
            ({"requirement_id": "pending", "workflow_id": "specification", "transaction_id": "20260915T103000Z-a1b2c3d4"}, "reserved_transaction_owner"),
            ({"requirement_id": "feature-1", "workflow_id": "../specification", "transaction_id": "20260915T103000Z-a1b2c3d4"}, "invalid_workflow_id"),
            ({"requirement_id": "feature-1", "workflow_id": "specification", "transaction_id": "specification-001"}, "invalid_transaction_id"),
        )
        for arguments, expected_code in cases:
            with self.subTest(arguments=arguments):
                with self.assertRaises(WorkError) as context:
                    transaction_directory(self.project_root, **arguments)
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

    def test_resolves_relative_path_from_uncanonicalized_project_root(self) -> None:
        project_root = Path(self.temporary_directory.name)

        normalized, resolved = resolve_project_relative_path(
            project_root,
            "outputs/work/task.json",
        )

        self.assertEqual(normalized, "outputs/work/task.json")
        self.assertEqual(
            resolved,
            project_root.resolve() / "outputs" / "work" / "task.json",
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

        self.assertEqual(artifacts["task"], "outputs/work/tasks/feature-1/index.json")
        self.assertEqual(
            validate_artifact_paths(
                self.project_root,
                "feature-1",
                artifacts,
                actual_plan_path=artifacts["plan"],
                allow_task_index=True,
            ),
            artifacts,
        )

    def test_collection_defaults_are_active(self) -> None:
        default = default_artifact_paths(self.project_root, "feature-1")
        collection = default_task_collection_artifact_paths(
            self.project_root, "feature-1"
        )

        self.assertEqual(default["task"], "outputs/work/tasks/feature-1/index.json")
        self.assertEqual(
            collection,
            {
                "plan": "outputs/work/plans/feature-1.json",
                "task": "outputs/work/tasks/feature-1/index.json",
                "execution": "outputs/work/executions/feature-1",
            },
        )

    def test_validates_default_and_nondefault_task_collection_index_paths(self) -> None:
        for raw in (
            "outputs/work/tasks/feature-1/index.json",
            "custom/feature-1/index.json",
        ):
            with self.subTest(raw=raw):
                normalized, resolved = validate_task_collection_index_path(
                    self.project_root, "feature-1", raw
                )
                self.assertEqual(normalized, raw)
                self.assertEqual(
                    resolved, self.project_root.joinpath(*raw.split("/"))
                )

        for raw in (
            "outputs/work/tasks/feature-1/task.json",
            "outputs/work/tasks/other/index.json",
            "outputs/work/tasks/feature-1/tasks/index.json",
        ):
            with self.subTest(raw=raw):
                with self.assertRaises(WorkError) as context:
                    validate_task_collection_index_path(
                        self.project_root, "feature-1", raw
                    )
                self.assertEqual(
                    context.exception.code,
                    "task_index_path_requirement_mismatch",
                )

    def test_resolves_only_exact_task_item_path(self) -> None:
        item_path = "tasks/TASK-001.json"
        normalized, resolved = resolve_task_collection_item_path(
            self.project_root,
            "feature-1",
            "outputs/work/tasks/feature-1/index.json",
            "TASK-001",
            item_path,
        )

        self.assertEqual(normalized, item_path)
        self.assertEqual(
            resolved,
            self.project_root
            / "outputs"
            / "work"
            / "tasks"
            / "feature-1"
            / "tasks"
            / "TASK-001.json",
        )

        cases = (
            ("task-001", item_path, "invalid_task_item_id"),
            ("TASK-001", r"tasks\TASK-001.json", "task_item_path_mismatch"),
            ("TASK-001", "tasks/task-001.json", "task_item_path_mismatch"),
            ("TASK-001", "TASK-001.json", "task_item_path_mismatch"),
            ("TASK-001", "../TASK-001.json", "task_item_path_mismatch"),
            ("TASK-001", "tasks/TASK-002.json", "task_item_path_mismatch"),
        )
        for task_id, raw, expected_code in cases:
            with self.subTest(task_id=task_id, raw=raw):
                with self.assertRaises(WorkError) as context:
                    resolve_task_collection_item_path(
                        self.project_root,
                        "feature-1",
                        "outputs/work/tasks/feature-1/index.json",
                        task_id,
                        raw,
                    )
                self.assertEqual(context.exception.code, expected_code)

    def test_task_item_path_rejects_link_outside_collection(self) -> None:
        collection = self.project_root / "outputs" / "work" / "tasks" / "feature-1"
        elsewhere = self.project_root / "elsewhere"
        collection.mkdir(parents=True)
        elsewhere.mkdir()
        try:
            (collection / "tasks").symlink_to(elsewhere, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"directory symlinks unavailable: {error}")

        with self.assertRaises(WorkError) as context:
            resolve_task_collection_item_path(
                self.project_root,
                "feature-1",
                "outputs/work/tasks/feature-1/index.json",
                "TASK-001",
                "tasks/TASK-001.json",
            )

        self.assertEqual(
            context.exception.code,
            "task_item_path_escapes_collection",
        )

    def test_task_item_paths_reject_portable_aliases(self) -> None:
        with self.assertRaises(WorkError) as context:
            validate_task_item_path_aliases(
                {
                    "TASK-001": self.project_root / "tasks" / "CAFÉ.json",
                    "TASK-002": self.project_root
                    / "TASKS"
                    / "cafe\N{COMBINING ACUTE ACCENT}.json",
                }
            )

        self.assertEqual(context.exception.code, "task_item_path_alias")

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
