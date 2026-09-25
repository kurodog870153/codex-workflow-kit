<!-- work-compatibility-revision: 1 -->
# Resolve roots and runtime


1. Resolve `<skill-root>` as the directory containing the active `work/SKILL.md`.
2. Resolve the project root as the root of the Git repository containing the current working directory. If the current working directory is not inside a Git repository, use the current working directory.
3. Resolve `<work-cli>` from that exact `<skill-root>`: `"<skill-root>/scripts/work"` on macOS, or `"<skill-root>\scripts\work.exe"` on Windows. Use the absolute installed path, never a PATH lookup or another skill installation.
4. Before the first Work CLI command, run `<work-cli> --help` and require a successful `work-cli-result/v1` response. If the binary is missing or cannot start, stop and report the installed path.
5. Use the same resolved `<work-cli>` throughout the current workflow. Pass `--project-root "<project-root>"` where the command requires it; request file paths may be absolute or relative to the process working directory.
