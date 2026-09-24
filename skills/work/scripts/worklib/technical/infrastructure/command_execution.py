from __future__ import annotations

import subprocess
from threading import Thread


class _TailCapture:
    def __init__(self) -> None:
        self.size = 0
        self.tail = b""

    def drain(self, stream) -> None:
        try:
            while chunk := stream.read(65536):
                self.size += len(chunk)
                self.tail = (self.tail + chunk)[-4096:]
        finally:
            stream.close()


def execute_command(argv: list[str], cwd: str, timeout: int) -> dict[str, object]:
    streams = {name: _TailCapture() for name in ("stdout", "stderr")}
    try:
        with subprocess.Popen(
            argv, cwd=cwd, shell=False, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        ) as process:
            assert process.stdout is not None and process.stderr is not None
            readers = [
                Thread(target=streams[name].drain, args=(pipe,), daemon=True)
                for name, pipe in (("stdout", process.stdout), ("stderr", process.stderr))
            ]
            for reader in readers:
                reader.start()
            try:
                exit_code = process.wait(timeout=timeout)
                result: dict[str, object] = {"status": "exited", "exit_code": exit_code}
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                result = {"status": "timed_out", "exit_code": None}
            for reader in readers:
                reader.join()
    except OSError:
        result = {"status": "launch_failed", "exit_code": None}
    for name, captured in streams.items():
        result[name + "_tail"] = captured.tail.decode("utf-8", errors="replace")
        result[name + "_truncated"] = captured.size > 4096
    return result

