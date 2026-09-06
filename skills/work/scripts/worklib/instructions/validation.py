from __future__ import annotations

import re
from typing import Any

from ..foundation.errors import ExitCode, WorkError


SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
SOURCE_FIELDS = {"kind", "logical_name", "canonical_sha256"}


def strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "expected_object",
            "A JSON object is required.",
            {"location": location},
        )
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - required)
    if missing or unknown:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            {"location": location, "missing": missing, "unknown": unknown},
        )
    return value


def string_array(
    value: object,
    *,
    location: str,
    allow_empty: bool,
) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A string array with the required cardinality is required.",
            {"location": location},
        )
    result: list[str] = []
    for index, item in enumerate(value):
        if not isinstance(item, str) or not item:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_string_array",
                "Every array item must be a non-empty string.",
                {"location": f"{location}[{index}]"},
            )
        result.append(item)
    return result


def sha256(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_sha256",
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            {"location": location},
        )
    return value
