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
from worklib.skills.catalog import SkillRoot, snapshot_catalog_skill
from worklib.skills.selection import selection_sha256


class SkillSelectionCliTests(unittest.TestCase):
    def test_selection_validate_command(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            project = base / "project"
            root_path = base / "skills"
            skill = root_path / "ui"
            project.mkdir()
            skill.mkdir(parents=True)
            (skill / "SKILL.md").write_text(
                "---\nname: ui\ndescription: Design interfaces.\n---\nInstructions\n",
                encoding="utf-8",
            )
            root = SkillRoot("repo", ".agents/skills", root_path)
            snapshot = snapshot_catalog_skill(root, "ui/SKILL.md")
            summary = snapshot["skill"]
            bundle = snapshot["bundle"]
            selected = {
                "id": summary["id"],
                "name": summary["name"],
                "scope": summary["scope"],
                "root": summary["root"],
                "source": summary["source"],
                "description": summary["description"],
                "mode_support": {
                    "plan": "inferred",
                    "task": "inferred",
                    "execute": "unsupported",
                },
                "allow_implicit_invocation": summary["allow_implicit_invocation"],
                "dependency_status": "available",
                "summary_sha256": summary["summary_sha256"],
                "bundle_sha256": bundle["bundle_sha256"],
                "recommendation_reason": "The request needs interface design.",
            }
            contract = {
                "schema": "work-skill-selection/v1",
                "decision": "external_skills",
                "skills": [selected],
                "selection_sha256": selection_sha256("external_skills", [selected]),
            }
            stdout = io.StringIO()
            stderr = io.StringIO()
            exit_code = main(
                [
                    "--project-root",
                    str(project),
                    "skills",
                    "selection-validate",
                    "--root",
                    f"repo:.agents/skills={root_path}",
                    "--stdin",
                ],
                stdin=io.StringIO(json.dumps(contract)),
                stdout=stdout,
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.SUCCESS)
        self.assertEqual(stderr.getvalue(), "")
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["schema"], "work-skill-selection-validation/v1")
        self.assertEqual(result["status"], "valid")

    def test_invalid_json_uses_work_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            stderr = io.StringIO()
            exit_code = main(
                [
                    "--project-root",
                    temporary,
                    "skills",
                    "selection-validate",
                    "--root",
                    f"repo:.agents/skills={temporary}",
                    "--stdin",
                ],
                stdin=io.StringIO("{"),
                stdout=io.StringIO(),
                stderr=stderr,
            )

        self.assertEqual(exit_code, ExitCode.INPUT_FORMAT)
        self.assertEqual(json.loads(stderr.getvalue())["code"], "invalid_json")


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
