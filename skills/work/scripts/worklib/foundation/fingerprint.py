from __future__ import annotations

import hashlib
import unicodedata
from collections.abc import Iterable
from pathlib import Path

from .errors import ExitCode, WorkError


UTF8_BOM = b"\xef\xbb\xbf"
INSTRUCTION_SOURCE_KINDS = frozenset({"workflow", "instruction", "reference"})


def decode_utf8(raw: bytes, *, source: str) -> str:
    if raw.startswith(UTF8_BOM):
        raw = raw[len(UTF8_BOM) :]

    try:
        return raw.decode("utf-8", errors="strict")
    except UnicodeDecodeError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            {"source": source, "byte_offset": error.start},
        ) from error


def canonical_text(text: str) -> str:
    normalized = unicodedata.normalize("NFC", text)
    normalized = normalized.replace("\r\n", "\n").replace("\r", "\n")
    return normalized.rstrip("\n") + "\n"


def canonical_bytes(raw: bytes, *, source: str) -> bytes:
    return canonical_text(decode_utf8(raw, source=source)).encode("utf-8")


def canonical_sha256(raw: bytes, *, source: str) -> str:
    return hashlib.sha256(canonical_bytes(raw, source=source)).hexdigest()


def raw_sha256(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def rules_sha256(
    scope: str,
    sources: Iterable[tuple[str, str, str, bytes]],
) -> str:
    if scope not in {"plan", "task", "execute"}:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_rule_scope",
            "The rule fingerprint scope is invalid.",
            {"scope": scope},
        )

    framed = bytearray(b"WORK-RULES-SHA-256-V1\n")
    for layer, kind, logical_name, content in sources:
        framed.extend(b"S")
        for value in (
            scope.encode("utf-8"),
            layer.encode("utf-8"),
            kind.encode("utf-8"),
            logical_name.encode("utf-8"),
            content,
        ):
            framed.extend(str(len(value)).encode("ascii"))
            framed.extend(b":")
            framed.extend(value)
        framed.extend(b"\n")
    framed.extend(b"END\n")
    return hashlib.sha256(framed).hexdigest()


def instructions_sha256(
    scope: str,
    sources: Iterable[tuple[str, str, bytes]],
) -> str:
    if scope not in {"plan", "task", "execute"}:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_instruction_scope",
            "The instruction fingerprint scope is invalid.",
            {"scope": scope},
        )

    framed = bytearray(b"WORK-INSTRUCTIONS-SHA-256-V1\n")
    for kind, logical_name, content in sources:
        if kind not in INSTRUCTION_SOURCE_KINDS:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_kind",
                "The instruction source kind is invalid.",
                {"kind": kind},
            )
        if not isinstance(logical_name, str) or not logical_name:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_logical_name",
                "The instruction source logical name must be a non-empty string.",
            )
        if not isinstance(content, bytes):
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_instruction_content",
                "Canonical instruction source content must be bytes.",
                {"logical_name": logical_name},
            )

        framed.extend(b"S")
        for value in (
            scope.encode("utf-8"),
            kind.encode("utf-8"),
            logical_name.encode("utf-8"),
            content,
        ):
            framed.extend(str(len(value)).encode("ascii"))
            framed.extend(b":")
            framed.extend(value)
        framed.extend(b"\n")
    framed.extend(b"END\n")
    return hashlib.sha256(framed).hexdigest()


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
