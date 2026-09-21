from __future__ import annotations

from pathlib import PurePosixPath

from ...models.common.errors import ExitCode, WorkError


def _decode_git_path(raw: bytes) -> str:
    try:
        path = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise WorkError(ExitCode.INPUT_FORMAT, "execute_worktree_invalid_utf8_path", "Git returned a path that is not valid UTF-8.") from error
    pure = PurePosixPath(path)
    if not path or pure.is_absolute() or any(part in {"", ".", ".."} for part in pure.parts):
        raise WorkError(ExitCode.INPUT_FORMAT, "execute_worktree_invalid_path", "Git returned an invalid project-relative path.", {"path": path})
    return path


def parse_porcelain_v1_z(raw: bytes) -> list[dict[str, str]]:
    parts = raw.split(b"\0")
    if parts and parts[-1] == b"":
        parts.pop()
    records: list[dict[str, str]] = []
    position = 0
    while position < len(parts):
        record = parts[position]
        if len(record) < 4 or record[2:3] != b" ":
            raise WorkError(ExitCode.INPUT_FORMAT, "execute_worktree_invalid_porcelain", "Git returned malformed porcelain v1 data.")
        try:
            index_status = record[0:1].decode("ascii")
            worktree_status = record[1:2].decode("ascii")
        except UnicodeDecodeError as error:
            raise WorkError(ExitCode.INPUT_FORMAT, "execute_worktree_invalid_porcelain", "Git returned a non-ASCII porcelain status.") from error
        item = {"index_status": index_status, "worktree_status": worktree_status, "path": _decode_git_path(record[3:])}
        position += 1
        if index_status in {"R", "C"} or worktree_status in {"R", "C"}:
            if position >= len(parts):
                raise WorkError(ExitCode.INPUT_FORMAT, "execute_worktree_invalid_porcelain", "A Git rename or copy record is incomplete.")
            item["original_path"] = _decode_git_path(parts[position])
            position += 1
        records.append(item)
    return records
