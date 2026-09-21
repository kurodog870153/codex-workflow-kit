from __future__ import annotations

import hashlib
import json
import unicodedata
from collections.abc import Iterable

UTF8_BOM = b"\xef\xbb\xbf"


def decode_utf8(raw: bytes, *, source: str) -> str:
    if raw.startswith(UTF8_BOM):
        raw = raw[len(UTF8_BOM) :]
    return raw.decode("utf-8", errors="strict")


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


def canonical_json_sha256(value: object) -> str:
    encoded = json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def instructions_sha256(
    scope: str,
    sources: Iterable[tuple[str, str, bytes]],
) -> str:
    framed = bytearray(b"WORK-INSTRUCTIONS-SHA-256-V1\n")
    for kind, logical_name, content in sources:
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
