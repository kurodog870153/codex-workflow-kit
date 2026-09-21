from __future__ import annotations

import os
import subprocess
import tempfile


def execute_command(argv: list[str], cwd: str, timeout: int) -> dict[str, object]:
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        try:
            process = subprocess.run(
                argv,
                cwd=cwd,
                shell=False,
                stdin=subprocess.DEVNULL,
                stdout=stdout,
                stderr=stderr,
                timeout=timeout,
                check=False,
            )
            result: dict[str, object] = {
                "status": "exited",
                "exit_code": process.returncode,
            }
        except subprocess.TimeoutExpired:
            result = {"status": "timed_out", "exit_code": None}
        except OSError:
            result = {"status": "launch_failed", "exit_code": None}
        for name, stream in (("stdout", stdout), ("stderr", stderr)):
            size = stream.seek(0, os.SEEK_END)
            stream.seek(max(0, size - 4096))
            result[name + "_tail"] = stream.read().decode("utf-8", errors="replace")
            result[name + "_truncated"] = size > 4096
        return result

