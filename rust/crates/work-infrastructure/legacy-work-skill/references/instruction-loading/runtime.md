<!-- work-compatibility-revision: 1 -->
# Resolve roots and runtime


1. Resolve `<skill-root>` as the directory containing the active `work/SKILL.md`.
2. Resolve the project root as the root of the Git repository containing the current working directory. If the current working directory is not inside a Git repository, use the current working directory.
3. Treat `"<skill-root>/scripts/work.py"` as the only Work Python CLI entry point. Do not substitute another module or script.
4. Before the first Work CLI invocation, resolve `<python-command>` to an available Python command prefix such as `py -3`, `python3`, or `python`, according to the current platform and environment.
5. Run `<python-command> -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 14) else 1)"` and require exit code `0` before invoking the Work CLI. Then run `<python-command> -c "import pydantic"` and require exit code `0` to confirm the user-installed dependency without changing it.
6. If no Python 3.14 or newer command or no Pydantic installation is available, stop and report the missing runtime or dependency. Do not install or upgrade Python packages without separate user authorization.
7. Use the same resolved `<python-command>` throughout the current workflow and invoke the CLI as `<python-command> "<skill-root>/scripts/work.py" ...`. Do not rely on file association or executable permission.
