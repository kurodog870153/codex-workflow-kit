"""Low-level file reads and file fingerprint adapters."""

from pathlib import Path

from ...models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import raw_sha256
from .text_codec import canonical_sha256, decode_utf8


def read_raw(path: Path) -> bytes:
    if not path.is_file():
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "file_not_found",
            "The required file does not exist or is not a regular file.",
            {"path": str(path)},
        )
    try:
        return path.read_bytes()
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "file_read_failed",
            "The file could not be read.",
            {"path": str(path)},
        ) from error


def fingerprint_file(path: Path) -> dict[str, str]:
    raw = read_raw(path)
    decode_utf8(raw, source=str(path))
    return {
        "canonical_sha256": canonical_sha256(raw, source=str(path)),
        "raw_sha256": raw_sha256(raw),
    }


__all__ = ["fingerprint_file", "read_raw"]
