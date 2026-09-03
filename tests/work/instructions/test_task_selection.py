from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.instructions.selection import build_instruction_selection
from worklib.instructions.task_selection import (
    build_task_document_instruction_selection,
    validate_task_document_instruction_selection,
)


class TaskInstructionSelectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.skill_root = Path(self.temporary_directory.name) / "work"
        self._write("references/instruction-loading.md", b"loading\n")
        self._write("references/workflows/task.md", b"workflow\n")
        for hierarchy_path in (
            "general",
            "web",
            "web/backend",
            "web/backend/java",
            "web/backend/java/jpa",
            "web/backend/java/mybatis",
        ):
            self._write(
                f"references/instructions/task/{hierarchy_path}/instructions.md",
                (
                    "---\n"
                    f"name: {hierarchy_path}\n"
                    f"description: {hierarchy_path} instructions.\n"
                    "metadata:\n"
                    "  work-tags:\n"
                    "    - test-tag\n"
                    "---\n\n"
                    f"{hierarchy_path}\n"
                ).encode("utf-8"),
            )
        self._write(
            "references/instructions/task/general/references/task-records.md",
            b"records\n",
        )
        self._write(
            "references/instructions/task/web/backend/references/security.md",
            b"security\n",
        )
        self._write(
            "references/instructions/task/web/backend/java/references/relational-data.md",
            b"relational\n",
        )
        self.task_selections = [
            build_instruction_selection(
                skill_root=self.skill_root,
                mode="task",
                selected_paths=["web/backend/java/jpa"],
                reference_names=[
                    "task.general.task-records",
                    "task.web.backend.java.relational-data",
                ],
            ),
            build_instruction_selection(
                skill_root=self.skill_root,
                mode="task",
                selected_paths=["web/backend/java/mybatis"],
                reference_names=[
                    "task.general.task-records",
                    "task.web.backend.security",
                ],
            ),
        ]

    def _write(self, relative_path: str, content: bytes) -> None:
        path = self.skill_root / Path(*relative_path.split("/"))
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)

    def _document_selection(self) -> dict[str, object]:
        return build_task_document_instruction_selection(
            self.task_selections,
            skill_root=self.skill_root,
        )

    def test_builder_uses_first_occurrence_union(self) -> None:
        selection = self._document_selection()

        self.assertEqual(
            [source["logical_name"] for source in selection["sources"]],  # type: ignore[index]
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.general.task-records",
                "task.web",
                "task.web.backend",
                "task.web.backend.java",
                "task.web.backend.java.relational-data",
                "task.web.backend.java.jpa",
                "task.web.backend.security",
                "task.web.backend.java.mybatis",
            ],
        )
        self.assertEqual(
            selection["references"],
            [
                "task.general.task-records",
                "task.web.backend.java.relational-data",
                "task.web.backend.security",
            ],
        )
        self.assertEqual(len(selection["instructions_sha256"]), 64)  # type: ignore[arg-type]

    def test_builder_output_round_trips_through_validator(self) -> None:
        selection = self._document_selection()

        validated = validate_task_document_instruction_selection(
            selection,
            self.task_selections,
            skill_root=self.skill_root,
        )

        self.assertEqual(validated, selection)

    def test_document_source_layer_is_rejected(self) -> None:
        selection = self._document_selection()
        sources = selection["sources"]
        assert isinstance(sources, list) and isinstance(sources[0], dict)
        sources[0]["layer"] = "project"

        with self.assertRaises(WorkError) as context:
            validate_task_document_instruction_selection(
                selection,
                self.task_selections,
                skill_root=self.skill_root,
            )
        self.assertEqual(context.exception.code, "invalid_object_fields")
        self.assertEqual(context.exception.details["unknown"], ["layer"])

    def test_document_source_order_mismatch_is_rejected(self) -> None:
        selection = self._document_selection()
        sources = selection["sources"]
        assert isinstance(sources, list)
        sources[0], sources[1] = sources[1], sources[0]

        with self.assertRaises(WorkError) as context:
            validate_task_document_instruction_selection(
                selection,
                self.task_selections,
                skill_root=self.skill_root,
            )
        self.assertEqual(
            context.exception.code,
            "task_document_instruction_sources_mismatch",
        )

    def test_document_reference_order_mismatch_is_rejected(self) -> None:
        selection = copy.deepcopy(self._document_selection())
        references = selection["references"]
        assert isinstance(references, list)
        references.reverse()

        with self.assertRaises(WorkError) as context:
            validate_task_document_instruction_selection(
                selection,
                self.task_selections,
                skill_root=self.skill_root,
            )
        self.assertEqual(
            context.exception.code,
            "task_document_instruction_references_mismatch",
        )

    def test_document_stale_fingerprint_is_rejected(self) -> None:
        selection = self._document_selection()
        selection["instructions_sha256"] = "0" * 64

        with self.assertRaises(WorkError) as context:
            validate_task_document_instruction_selection(
                selection,
                self.task_selections,
                skill_root=self.skill_root,
            )
        self.assertEqual(
            context.exception.code,
            "task_document_instructions_fingerprint_mismatch",
        )


if __name__ == "__main__":
    unittest.main()
