from __future__ import annotations

import json

from ....technical.foundation.fingerprint import canonical_text
from ....models.common.errors import ExitCode, WorkError


def reject(code: str, message: str, **details):
    raise WorkError(ExitCode.CONTRACT, code, message, details)


def json_document(text: str, raw: bytes):
    duplicates: list[str] = []

    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                duplicates.append(key)
            result[key] = value
        return result

    def constant(value):
        reject("invalid_json_constant", "JSON cannot contain non-standard numeric constants.")

    try:
        value = json.loads(text, object_pairs_hook=object_pairs, parse_constant=constant)
    except json.JSONDecodeError as error:
        bom_size = 3 if raw.startswith(b"\xef\xbb\xbf") else 0
        reject("invalid_json_contract", "The TASK JSON is invalid.",
                line=error.lineno, column=error.colno,
                byte_offset=bom_size + len(text[:error.pos].encode("utf-8")))
    except (RecursionError, ValueError):
        reject("invalid_json_contract", "The TASK JSON exceeds the parser's supported limits.")
    # Escaped unpaired surrogates cannot be rendered as UTF-8.
    try:
        json.dumps(value, ensure_ascii=False).encode("utf-8")
    except UnicodeEncodeError:
        reject("invalid_json_unicode", "JSON strings contain an unpaired Unicode surrogate.")
    if duplicates:
        reject("duplicate_json_key", "Duplicate keys are ambiguous; no parsed document will be used.",
                keys=sorted(set(duplicates)))
    if not isinstance(value, dict):
        reject("json_contract_not_object", "The TASK document must be a JSON object.")
    return value


def normalization(text, raw):
    if canonical_text(text).encode("utf-8") != raw:
        reject("noncanonical_task_text", "TASK text must be UTF-8 without BOM, NFC, LF and one trailing LF.")
    return True


def same(actual, expected, code, message):
    if actual != expected:
        reject(code, message, expected=expected, actual=actual)
    return True


__all__ = ["json_document", "normalization", "reject", "same"]


