from __future__ import annotations

import subprocess
from pathlib import Path

from ...models.common.errors import ExitCode, WorkError


GIT_TIMEOUT_SECONDS = 30


def run_read_only_git(project_root: Path, arguments: list[str]) -> bytes:
    try:
        result = subprocess.run(
            ["git", *arguments],
            cwd=project_root,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=GIT_TIMEOUT_SECONDS,
        )
    except FileNotFoundError as error:
        raise WorkError(ExitCode.IO_FAILURE, "execute_worktree_git_missing", "Git is not available.") from error
    except subprocess.TimeoutExpired as error:
        raise WorkError(ExitCode.IO_FAILURE, "execute_worktree_git_timeout", "The read-only Git command timed out.") from error
    if result.returncode != 0:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "execute_worktree_git_failed",
            "The read-only Git command failed.",
            {"exit_code": result.returncode},
        )
    return result.stdout

