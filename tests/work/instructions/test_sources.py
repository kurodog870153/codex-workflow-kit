from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.instructions.sources import load_instruction_sources


class InstructionSourceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.skill_root = Path(self.temporary_directory.name) / "work"

    def _write(self, relative_path: str, content: bytes) -> Path:
        path = self.skill_root / Path(*relative_path.split("/"))
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def _write_core(self, mode: str, hierarchy_paths: tuple[str, ...]) -> None:
        self._write(
            "references/instruction-loading.md",
            b"\xef\xbb\xbfloading\r\n\r\n",
        )
        self._write(f"references/workflows/{mode}.md", b"workflow\n")
        for hierarchy_path in hierarchy_paths:
            self._write(
                f"references/instructions/{mode}/{hierarchy_path}/instructions.md",
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

    def test_sources_follow_workflow_and_selected_hierarchy_order(self) -> None:
        self._write_core("task", ("general", "web", "web/backend"))

        loaded = load_instruction_sources(
            self.skill_root,
            "task",
            ["web/backend"],
        )

        self.assertEqual(
            [(source.kind, source.logical_name) for source in loaded.sources],
            [
                ("workflow", "work.instruction-loading"),
                ("workflow", "work.workflow.task"),
                ("instruction", "task.general"),
                ("instruction", "task.web"),
                ("instruction", "task.web.backend"),
            ],
        )
        self.assertEqual(loaded.sources[0].canonical_content, b"loading\n")
        self.assertEqual(len(loaded.instructions_sha256), 64)
        contract = loaded.as_dict()
        self.assertEqual(contract["schema"], "work-instructions/v1")
        self.assertEqual(
            set(contract["sources"][0]),  # type: ignore[index]
            {"kind", "logical_name", "canonical_sha256"},
        )

    def test_canonical_content_change_changes_fingerprint(self) -> None:
        workflow = self._write_core("plan", ("general",))
        first = load_instruction_sources(self.skill_root, "plan", [])
        workflow = self.skill_root / "references" / "workflows" / "plan.md"
        workflow.write_bytes(b"changed workflow\n")

        second = load_instruction_sources(self.skill_root, "plan", [])

        self.assertNotEqual(first.instructions_sha256, second.instructions_sha256)

    def test_invalid_utf8_is_rejected(self) -> None:
        self._write_core("execute", ("general",))
        self._write("references/workflows/execute.md", b"\xff")

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(self.skill_root, "execute", [])
        self.assertEqual(context.exception.code, "invalid_utf8")

    def test_missing_mode_workflow_is_rejected(self) -> None:
        self._write_core("task", ("general",))
        (self.skill_root / "references" / "workflows" / "task.md").unlink()

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(self.skill_root, "task", [])
        self.assertEqual(context.exception.code, "instruction_source_missing")
        self.assertEqual(
            context.exception.details["logical_name"],
            "work.workflow.task",
        )

    def test_workflow_symlink_cannot_escape_skill_root(self) -> None:
        self._write_core("plan", ("general",))
        workflow = self.skill_root / "references" / "workflows" / "plan.md"
        workflow.unlink()
        external_workflow = Path(self.temporary_directory.name) / "outside.md"
        external_workflow.write_bytes(b"outside\n")
        try:
            workflow.symlink_to(external_workflow)
        except OSError as error:
            self.skipTest(f"Symbolic links are unavailable: {error}")

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(self.skill_root, "plan", [])
        self.assertEqual(
            context.exception.code,
            "instruction_source_escapes_skill_root",
        )

    def test_references_follow_their_declaring_instruction(self) -> None:
        self._write_core("task", ("general", "web", "web/backend"))
        self._write(
            "references/instructions/task/general/references/task-records.md",
            b"records\n",
        )
        self._write(
            "references/instructions/task/web/backend/references/security.md",
            b"security\n",
        )
        self._write(
            "references/instructions/task/web/backend/references/resilience.md",
            b"resilience\n",
        )

        loaded = load_instruction_sources(
            self.skill_root,
            "task",
            ["web/backend"],
            [
                "task.web.backend.resilience",
                "task.general.task-records",
                "task.web.backend.security",
            ],
        )

        self.assertEqual(
            [source.logical_name for source in loaded.sources],
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.general.task-records",
                "task.web",
                "task.web.backend",
                "task.web.backend.resilience",
                "task.web.backend.security",
            ],
        )
        self.assertEqual(
            loaded.references,
            (
                "task.general.task-records",
                "task.web.backend.resilience",
                "task.web.backend.security",
            ),
        )
        self.assertEqual(loaded.as_dict()["references"], list(loaded.references))

    def test_duplicate_reference_is_rejected(self) -> None:
        self._write_core("plan", ("general",))
        reference = "plan.general.external-operations"

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(
                self.skill_root,
                "plan",
                [],
                [reference, reference],
            )
        self.assertEqual(context.exception.code, "duplicate_instruction_reference")

    def test_reference_outside_selected_hierarchy_is_rejected(self) -> None:
        self._write_core("task", ("general", "web", "web/backend"))

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(
                self.skill_root,
                "task",
                [],
                ["task.web.backend.security"],
            )
        self.assertEqual(
            context.exception.code,
            "unroutable_instruction_reference",
        )

    def test_missing_reference_is_rejected(self) -> None:
        self._write_core("execute", ("general",))

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(
                self.skill_root,
                "execute",
                [],
                ["execute.general.execution-records"],
            )
        self.assertEqual(context.exception.code, "instruction_source_missing")
        self.assertEqual(
            context.exception.details["logical_name"],
            "execute.general.execution-records",
        )

    def test_invalid_utf8_reference_is_rejected(self) -> None:
        self._write_core("task", ("general",))
        self._write(
            "references/instructions/task/general/references/task-records.md",
            b"\xff",
        )

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(
                self.skill_root,
                "task",
                [],
                ["task.general.task-records"],
            )
        self.assertEqual(context.exception.code, "invalid_utf8")

    def test_reference_symlink_cannot_escape_declaring_hierarchy(self) -> None:
        self._write_core("task", ("general", "web"))
        external_reference = self._write(
            "references/instructions/task/web/references/task-records.md",
            b"outside hierarchy\n",
        )
        reference = (
            self.skill_root
            / "references"
            / "instructions"
            / "task"
            / "general"
            / "references"
            / "task-records.md"
        )
        reference.parent.mkdir(parents=True)
        try:
            reference.symlink_to(external_reference)
        except OSError as error:
            self.skipTest(f"Symbolic links are unavailable: {error}")

        with self.assertRaises(WorkError) as context:
            load_instruction_sources(
                self.skill_root,
                "task",
                [],
                ["task.general.task-records"],
            )
        self.assertEqual(
            context.exception.code,
            "instruction_reference_escapes_hierarchy",
        )


if __name__ == "__main__":
    unittest.main()
