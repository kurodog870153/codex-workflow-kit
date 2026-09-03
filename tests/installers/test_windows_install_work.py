from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
INSTALLER = REPOSITORY_ROOT / "os-scripts" / "windows" / "install-work.bat"


@unittest.skipUnless(os.name == "nt", "Windows installer integration tests")
class WindowsWorkInstallerTests(unittest.TestCase):
    def test_installer_requires_pyyaml_without_installing_it(self) -> None:
        content = INSTALLER.read_text(encoding="utf-8")

        self.assertIn('!python_command! -c "import yaml" <nul', content)
        self.assertEqual(content.count("<nul >nul 2>nul"), 4)
        self.assertEqual(content.count("call :validate_pyyaml"), 3)
        self.assertNotIn("goto validate_pyyaml", content)
        self.assertIn("PyYAML is required.", content)
        self.assertIn("does not install Python packages automatically", content)
        self.assertNotIn("pip install", content.lower())

    def run_installer(
        self,
        user_input: str,
        *,
        user_profile: Path,
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment["USERPROFILE"] = str(user_profile)
        return subprocess.run(
            ["cmd.exe", "/d", "/c", str(INSTALLER)],
            cwd=REPOSITORY_ROOT,
            env=environment,
            input=user_input,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=60,
            check=False,
        )

    def assert_base_install(self, home: Path) -> Path:
        work = home / ".agents" / "skills" / "work"
        self.assertTrue((work / "SKILL.md").is_file())
        self.assertTrue((work / "agents" / "openai.yaml").is_file())
        self.assertTrue((work / "references" / "instruction-loading.md").is_file())
        for mode in ("plan", "task", "execute"):
            self.assertTrue((work / "references" / "workflows" / f"{mode}.md").is_file())
            self.assertTrue((work / "references" / "subagents" / f"{mode}.md").is_file())
        self.assertTrue((work / "scripts" / "work.py").is_file())
        self.assertTrue((work / "scripts" / "worklib" / "cli.py").is_file())
        self.assertTrue(
            (work / "scripts" / "worklib" / "hierarchy" / "selection.py").is_file()
        )
        self.assertFalse((work / "scripts" / "worklib" / "rules.py").exists())
        self.assertFalse((work / "scripts" / "tests").exists())
        self.assertFalse((work / "plan").exists())
        self.assertFalse((work / "task").exists())
        self.assertFalse((work / "execute").exists())
        self.assertFalse((work / "shared").exists())
        self.assertFalse((home / ".agents" / "agents").exists())
        self.assertFalse((home / ".agents" / "rules").exists())
        return work

    def test_default_home_can_install_only_through_backend(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="work-installer-default-",
            dir=REPOSITORY_ROOT / "tests",
        ) as directory:
            home = Path(directory)
            result = self.run_installer("1\n3\nx\n", user_profile=home)

            self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
            work = self.assert_base_install(home)
            for mode in ("plan", "task", "execute"):
                instruction_root = work / "references" / "instructions" / mode
                for relative in ("general", "web", "web/backend"):
                    self.assertTrue((instruction_root / relative / "instructions.md").is_file())
                self.assertFalse(
                    (instruction_root / "web/backend/java/instructions.md").exists()
                )

    def test_custom_home_can_install_jpa_and_mybatis(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="work-installer-custom-",
            dir=REPOSITORY_ROOT / "tests",
        ) as directory:
            home = Path(directory)
            result = self.run_installer(
                f"2\n{home}\n5 6\nx\n",
                user_profile=REPOSITORY_ROOT,
            )

            self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
            work = self.assert_base_install(home)
            plan_root = work / "references" / "instructions" / "plan"
            self.assertTrue((plan_root / "web/backend/java/instructions.md").is_file())
            self.assertFalse((plan_root / "web/backend/java/jpa").exists())
            self.assertFalse((plan_root / "web/backend/java/mybatis").exists())
            for mode in ("task", "execute"):
                java_root = (
                    work
                    / "references"
                    / "instructions"
                    / mode
                    / "web"
                    / "backend"
                    / "java"
                )
                self.assertTrue((java_root / "instructions.md").is_file())
                self.assertTrue((java_root / "jpa" / "instructions.md").is_file())
                self.assertTrue((java_root / "mybatis" / "instructions.md").is_file())

    def test_custom_home_can_install_astro_and_tailwind(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="work-installer-frontend-",
            dir=REPOSITORY_ROOT / "tests",
        ) as directory:
            home = Path(directory)
            result = self.run_installer(
                f"2\n{home}\n9 11\nx\n",
                user_profile=REPOSITORY_ROOT,
            )

            self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
            work = self.assert_base_install(home)
            plan_root = work / "references" / "instructions" / "plan" / "web" / "frontend"
            self.assertTrue((plan_root / "typescript" / "instructions.md").is_file())
            self.assertTrue((plan_root / "css" / "instructions.md").is_file())
            self.assertFalse((plan_root / "typescript" / "astro").exists())
            self.assertFalse((plan_root / "css" / "tailwind").exists())
            for mode in ("task", "execute"):
                root = work / "references" / "instructions" / mode / "web" / "frontend"
                self.assertTrue((root / "typescript" / "astro" / "instructions.md").is_file())
                self.assertTrue((root / "css" / "tailwind" / "instructions.md").is_file())


if __name__ == "__main__":
    unittest.main()
