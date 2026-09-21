"""UTF-8 and canonical text adapters with stable product errors."""

from ...models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import (
    canonical_bytes as _canonical_bytes,
    canonical_sha256 as _canonical_sha256,
    decode_utf8 as _decode_utf8,
)


def _invalid_utf8(error: UnicodeDecodeError, *, source: str) -> WorkError:
    return WorkError(
        ExitCode.INPUT_FORMAT,
        "invalid_utf8",
        "The input is not valid UTF-8.",
        {"source": source, "byte_offset": error.start},
    )


def decode_utf8(raw: bytes, *, source: str) -> str:
    try:
        return _decode_utf8(raw, source=source)
    except UnicodeDecodeError as error:
        raise _invalid_utf8(error, source=source) from error


def canonical_bytes(raw: bytes, *, source: str) -> bytes:
    try:
        return _canonical_bytes(raw, source=source)
    except UnicodeDecodeError as error:
        raise _invalid_utf8(error, source=source) from error


def canonical_sha256(raw: bytes, *, source: str) -> str:
    try:
        return _canonical_sha256(raw, source=source)
    except UnicodeDecodeError as error:
        raise _invalid_utf8(error, source=source) from error


__all__ = ["canonical_bytes", "canonical_sha256", "decode_utf8"]
