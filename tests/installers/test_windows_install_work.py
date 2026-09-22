from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
INSTALLER = REPOSITORY_ROOT / "os-scripts" / "windows" / "install-work.bat"


@unittest.skipUnless(os.name == "nt", "Windows installer integration tests")
class WindowsWorkInstallerTests(unittest.TestCase):
    def test_installer_requires_python_dependencies_without_installing_them(self) -> None:
        content = INSTALLER.read_text(encoding="utf-8")

        self.assertIn('!python_command! -c "import yaml" <nul', content)
        self.assertIn('!python_command! -c "import pydantic" <nul', content)
        self.assertIn("sys.version_info >= (3, 14)", content)
        self.assertEqual(content.count("<nul >nul 2>nul"), 5)
        self.assertEqual(content.count("call :validate_python_dependencies"), 3)
        self.assertIn("PyYAML is required.", content)
        self.assertIn("Pydantic is required.", content)
        self.assertIn("does not install Python packages automatically", content)
        self.assertNotIn("pip install", content.lower())

    def run_installer(
        self,
        user_input: str,
        *,
        user_profile: Path,
        installer: Path = INSTALLER,
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment["USERPROFILE"] = str(user_profile)
        # Consecutive SET /P prompts can lose buffered input from a pipe.
        with tempfile.TemporaryFile(mode="w+b") as input_stream:
            input_stream.write(user_input.replace("\n", "\r\n").encode("utf-8"))
            input_stream.seek(0)
            return subprocess.run(
                ["cmd.exe", "/d", "/c", str(installer)],
                cwd=REPOSITORY_ROOT,
                env=environment,
                stdin=input_stream,
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
        self.assertTrue((work / "references" / "instruction-loading" / "invocation.md").is_file())
        for mode in ("plan", "task", "execute"):
            self.assertTrue((work / "references" / "workflows" / f"{mode}.md").is_file())
        for mode in ("plan", "task-coordinator", "task-skill", "execute"):
            self.assertTrue((work / "references" / "subagents" / f"{mode}.md").is_file())
        self.assertTrue((work / "scripts" / "work.py").is_file())
        self.assertTrue((work / "scripts" / "worklib" / "cli.py").is_file())
        for relative in (
            "models/instruction/__init__.py",
            "models/instruction/catalog.py",
            "models/instruction/contracts.py",
            "models/instruction/source.py",
            "models/instruction/refresh.py",
            "models/hierarchy/__init__.py",
            "models/hierarchy/contracts.py",
            "services/hierarchy/__init__.py",
            "services/hierarchy/fingerprint.py",
            "services/hierarchy/ordering.py",
            "services/hierarchy/path.py",
            "services/hierarchy/selection.py",
            "services/hierarchy/validation.py",
            "business_services/hierarchy/__init__.py",
            "services/instruction/__init__.py",
            "services/instruction/catalog.py",
            "models/contract/__init__.py",
            "models/contract/catalog.py",
            "models/task_collection/repair.py",
            "services/contract/__init__.py",
            "services/contract/catalog.py",
            "business_services/contract/__init__.py",
            "services/task/draft/__init__.py",
            "services/task/draft/storage.py",
            "services/task/draft/validation.py",
            "services/task/task_diagnostics/__init__.py",
            "services/task/task_diagnostics/evaluation.py",
            "services/task/task_diagnostics/inspection.py",
            "services/task/task_diagnostics/report.py",
            "business_services/task/diagnostics.py",
            "services/instruction/hierarchy.py",
            "services/instruction/history.py",
            "services/instruction/root.py",
            "services/instruction/selection.py",
            "services/instruction/source.py",
            "services/instruction/task_selection.py",
            "services/instruction/validation.py",
            "services/instruction/work_selection.py",
            "business_services/instruction/__init__.py",
            "business_services/instruction/refresh.py",
            "models/plan/__init__.py",
            "models/plan/contracts.py",
            "models/task_collection/__init__.py",
            "models/task_collection/contracts.py",
            "models/execution/__init__.py",
            "models/execution/deviation.py",
            "models/execution/authorization.py",
            "services/attempt/__init__.py",
            "services/attempt/authorization.py",
            "models/execution/attempt.py",
            "services/attempt/validation.py",
            "models/execution/index.py",
            "services/plan/__init__.py",
            "services/plan/document.py",
            "services/plan/ordering.py",
            "services/plan/persistence/__init__.py",
            "services/plan/validation.py",
            "business_services/plan/__init__.py",
            "models/handoff/__init__.py",
            "models/handoff/contracts.py",
            "services/handoff/__init__.py",
            "services/handoff/build.py",
            "services/handoff/execution.py",
            "services/handoff/fingerprint.py",
            "services/handoff/source_io/__init__.py",
            "services/handoff/validation.py",
            "services/handoff/verification.py",
            "business_services/handoff/__init__.py",
        ):
            self.assertTrue((work / "scripts" / "worklib" / relative).is_file())
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
                user_profile=home / "unused-default-home",
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
                user_profile=home / "unused-default-home",
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



    def assert_install_contents(self, home: Path, branches: set[str] | None) -> Path:
        source = REPOSITORY_ROOT / "skills" / "work"
        expected = {}
        for path in source.rglob("*"):
            if not path.is_file() or "__pycache__" in path.parts or path.name == ".DS_Store":
                continue
            relative = path.relative_to(source)
            if relative.as_posix() == "scripts/worklib/rules.py" or path.suffix == ".pyc":
                continue
            if relative.parts[:2] == ("references", "instructions"):
                owner = path.parent
                while owner != source and not (owner / "instructions.md").is_file():
                    owner = owner.parent
                self.assertNotEqual(owner, source, msg=str(relative))
                branch = owner.relative_to(source / "references" / "instructions").parts[1:]
                if branches is not None and "/".join(branch) not in branches:
                    continue
            expected[relative.as_posix()] = path.read_bytes()
        work = home / ".agents" / "skills" / "work"
        actual = {
            path.relative_to(work).as_posix(): path.read_bytes()
            for path in work.rglob("*")
            if path.is_file()
        }
        self.assertEqual(set(actual), set(expected))
        for relative, content in expected.items():
            self.assertEqual(actual[relative], content, msg=relative)
        return work

    def test_install_contents_and_cli_outside_repository(self) -> None:
        cases = (
            ("1", {"general"}),
            ("all", None),
            ("3", {"general", "web", "web/backend"}),
            ("5 6", {
                "general", "web", "web/backend", "web/backend/java",
                "web/backend/java/jpa", "web/backend/java/mybatis",
            }),
            ("9 11", {
                "general", "web", "web/frontend", "web/frontend/typescript",
                "web/frontend/typescript/astro", "web/frontend/css",
                "web/frontend/css/tailwind",
            }),
        )
        for selection, branches in cases:
            with self.subTest(selection=selection), tempfile.TemporaryDirectory(
                prefix="work-installer-content-"
            ) as directory:
                home = Path(directory)
                result = self.run_installer(f"1\n{selection}\nx\n", user_profile=home)
                self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
                work = self.assert_install_contents(home, branches)
                environment = os.environ.copy()
                environment.pop("PYTHONPATH", None)
                result = subprocess.run(
                    [sys.executable, "-B", str(work / "scripts" / "work.py"), "--help"],
                    cwd=home,
                    env=environment,
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                    timeout=60,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
                self.assertIn("usage: work.py", result.stdout)

    def test_reinstall_keeps_branches_and_stale_files(self) -> None:
        with tempfile.TemporaryDirectory(prefix="work-installer-reinstall-") as directory:
            home = Path(directory)
            result = self.run_installer("1\nall\nx\n", user_profile=home)
            self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
            work = self.assert_install_contents(home, None)
            stale = work / "stale.txt"
            stale.write_text("keep this file", encoding="utf-8")
            (work / "SKILL.md").write_text("outdated", encoding="utf-8")
            result = self.run_installer("1\n1\nx\n", user_profile=home)
            self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
            self.assertIn("Previously installed branches and stale files will be kept", result.stdout)
            source = REPOSITORY_ROOT / "skills" / "work"
            for path in source.rglob("*"):
                if (
                    path.is_file()
                    and "__pycache__" not in path.parts
                    and path.suffix != ".pyc"
                    and path.name != ".DS_Store"
                ):
                    self.assertEqual(
                        (work / path.relative_to(source)).read_bytes(),
                        path.read_bytes(),
                        msg=str(path.relative_to(source)),
                    )
            self.assertEqual(stale.read_text(encoding="utf-8"), "keep this file")

    def test_missing_sources_do_not_write_target(self) -> None:
        missing_files = (
            "references/workflows/specification.md",
            "references/workflows/task-drafts.md",
            "references/workflows/repair.md",
            "references/workflows/progress.md",
            "references/subagents/artifact-editor.md",
            "references/subagents/progress-saver.md",
            "references/instructions/task/web/backend/instructions.md",
        )
        for missing in missing_files:
            for existing in (False, True):
                with self.subTest(missing=missing, existing=existing), tempfile.TemporaryDirectory(
                    prefix="work-installer-missing-"
                ) as directory:
                    root = Path(directory)
                    fixture = root / "source"
                    installer = fixture / INSTALLER.relative_to(REPOSITORY_ROOT)
                    installer.parent.mkdir(parents=True)
                    shutil.copy2(INSTALLER, installer)
                    source = REPOSITORY_ROOT / "skills" / "work"
                    for path in source.rglob("*"):
                        if not path.is_file() or "__pycache__" in path.parts or path.name == ".DS_Store":
                            continue
                        relative = path.relative_to(source)
                        if relative.as_posix() == missing:
                            continue
                        target = fixture / "skills" / "work" / relative
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(path, target)
                    home = root / "home"
                    home.mkdir()
                    work = home / ".agents" / "skills" / "work"
                    if existing:
                        work.mkdir(parents=True)
                        (work / "SKILL.md").write_bytes(b"existing installation")
                    result = self.run_installer(
                        "1\n3\nx\n", user_profile=home, installer=installer
                    )
                    self.assertEqual(result.returncode, 1, msg=result.stdout + result.stderr)
                    self.assertIn("required Work skill source not found", result.stdout + result.stderr)
                    self.assertIn(missing.replace("/", os.sep), result.stdout + result.stderr)
                    if existing:
                        self.assertEqual(list(work.iterdir()), [work / "SKILL.md"])
                        self.assertEqual((work / "SKILL.md").read_bytes(), b"existing installation")
                    else:
                        self.assertFalse((home / ".agents").exists())


if __name__ == "__main__":
    unittest.main()
