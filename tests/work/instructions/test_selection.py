from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.instructions.selection import (
    build_instruction_selection,
    validate_instruction_selection,
)


class InstructionSelectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.skill_root = Path(self.temporary_directory.name) / "work"
        self._write("references/instruction-loading.md", b"loading\n")
        self._write("references/workflows/task.md", b"workflow\n")
        self._write(
            "references/instructions/task/general/instructions.md",
            b"general\n",
        )
        self._write(
            "references/instructions/task/general/references/task-records.md",
            b"records\n",
        )
        self._write(
            "references/instructions/task/web/instructions.md",
            b"web\n",
        )
        self._write(
            "references/instructions/task/web/references/security.md",
            b"security\n",
        )
        for hierarchy_path in (
            "web/backend",
            "web/backend/java",
            "web/backend/java/jpa",
            "web/backend/java/mybatis",
        ):
            self._write(
                f"references/instructions/task/{hierarchy_path}/instructions.md",
                f"{hierarchy_path}\n".encode("utf-8"),
            )

    def _write(self, relative_path: str, content: bytes) -> None:
        path = self.skill_root / Path(*relative_path.split("/"))
        path.parent.mkdir(parents=True, exist_ok=True)
        if relative_path.endswith("/instructions.md") and not content.startswith(b"---"):
            content = (
                b"---\n"
                b"name: Test\n"
                b"description: Test instructions.\n"
                b"metadata:\n"
                b"  work-tags:\n"
                b"    - test-tag\n"
                b"---\n\n"
                + content
            )
        path.write_bytes(content)

    def _selection(self) -> dict[str, object]:
        return build_instruction_selection(
            skill_root=self.skill_root,
            mode="task",
            selected_paths=["web"],
            reference_names=["task.general.task-records", "task.web.security"],
        )

    def test_valid_selection_matches_current_installed_sources(self) -> None:
        selection = self._selection()

        current = validate_instruction_selection(
            selection,
            skill_root=self.skill_root,
            mode="task",
        )

        self.assertEqual(
            current.instructions_sha256,
            selection["instructions_sha256"],
        )

    def test_source_layer_field_is_rejected(self) -> None:
        selection = self._selection()
        sources = selection["sources"]
        assert isinstance(sources, list) and isinstance(sources[0], dict)
        sources[0]["layer"] = "project"

        with self.assertRaises(WorkError) as context:
            validate_instruction_selection(
                selection,
                skill_root=self.skill_root,
                mode="task",
            )
        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["unknown"], ["layer"])

    def test_source_order_mismatch_is_rejected(self) -> None:
        selection = self._selection()
        sources = selection["sources"]
        assert isinstance(sources, list)
        sources[0], sources[1] = sources[1], sources[0]

        with self.assertRaises(WorkError) as context:
            validate_instruction_selection(
                selection,
                skill_root=self.skill_root,
                mode="task",
            )
        self.assertEqual(
            context.exception.code,
            "instruction_selection_sources_mismatch",
        )

    def test_stale_fingerprint_is_rejected(self) -> None:
        selection = self._selection()
        selection["instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            validate_instruction_selection(
                selection,
                skill_root=self.skill_root,
                mode="task",
            )
        self.assertEqual(context.exception.code, "instructions_fingerprint_mismatch")

    def test_reference_order_mismatch_is_rejected(self) -> None:
        selection = copy.deepcopy(self._selection())
        references = selection["references"]
        assert isinstance(references, list)
        references.reverse()

        with self.assertRaises(WorkError) as context:
            validate_instruction_selection(
                selection,
                skill_root=self.skill_root,
                mode="task",
            )
        self.assertEqual(
            context.exception.code,
            "instruction_selection_references_mismatch",
        )

    def test_builder_has_stable_fields_and_actual_reference_order(self) -> None:
        selection = build_instruction_selection(
            skill_root=self.skill_root,
            mode="task",
            selected_paths=["web"],
            reference_names=["task.web.security", "task.general.task-records"],
        )

        self.assertEqual(
            list(selection),
            [
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256",
            ],
        )
        self.assertEqual(
            selection["references"],
            ["task.general.task-records", "task.web.security"],
        )
        validate_instruction_selection(
            selection,
            skill_root=self.skill_root,
            mode="task",
        )

    def test_builder_preserves_ordered_leaf_paths(self) -> None:
        selection = build_instruction_selection(
            skill_root=self.skill_root,
            mode="task",
            selected_paths=["web/backend/java/jpa", "web/backend/java/mybatis"],
        )

        self.assertEqual(
            selection["selected_paths"],
            ["web/backend/java/jpa", "web/backend/java/mybatis"],
        )
        self.assertEqual(
            selection["resolved_paths"],
            [
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
            ],
        )
        validate_instruction_selection(
            selection,
            skill_root=self.skill_root,
            mode="task",
        )


if __name__ == "__main__":
    unittest.main()
