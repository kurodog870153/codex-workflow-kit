from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import main
from worklib.foundation.errors import ExitCode


class SkillCatalogCliTests(unittest.TestCase):
    def test_catalog_and_snapshot_commands(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary) / "project"
            root = Path(temporary) / "skills"
            project.mkdir()
            skill = root / "frontend"
            skill.mkdir(parents=True)
            (skill / "SKILL.md").write_text(
                "---\nname: frontend\ndescription: Build frontends.\n---\nInstructions\n",
                encoding="utf-8",
            )
            root_argument = f"repo:.agents/skills={root}"
            stdout = io.StringIO()
            stderr = io.StringIO()

            exit_code = main(
                ["--project-root", str(project), "skills", "catalog", "--root", root_argument],
                stdout=stdout,
                stderr=stderr,
            )
            catalog = json.loads(stdout.getvalue())
            self.assertEqual(exit_code, ExitCode.SUCCESS)
            self.assertEqual(stderr.getvalue(), "")
            self.assertEqual(catalog["schema"], "work-skill-catalog/v1")
            self.assertEqual(catalog["skills"][0]["name"], "frontend")

            stdout = io.StringIO()
            exit_code = main(
                [
                    "--project-root",
                    str(project),
                    "skills",
                    "snapshot",
                    "--root",
                    root_argument,
                    "--source",
                    "frontend/SKILL.md",
                ],
                stdout=stdout,
                stderr=io.StringIO(),
            )
            snapshot = json.loads(stdout.getvalue())
            self.assertEqual(exit_code, ExitCode.SUCCESS)
            self.assertEqual(snapshot["schema"], "work-skill-snapshot/v1")
            self.assertEqual(len(snapshot["bundle"]["bundle_sha256"]), 64)

    def test_invalid_root_argument_uses_work_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stdout = io.StringIO()
            stderr = io.StringIO()
            exit_code = main(
                ["--project-root", temporary, "skills", "catalog", "--root", "repo"],
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.CLI_USAGE)
        self.assertEqual(stdout.getvalue(), "")
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["schema"], "work-error/v1")
        self.assertEqual(error["code"], "invalid_skill_root_argument")


if __name__ == "__main__":
    unittest.main()
