"""Shared value-validation policy for Work contracts."""

from __future__ import annotations

import re
from typing import Any

from ...protocol import INVALID_SHA256_ERROR_CODE, SHA256_PATTERN
from .errors import ExitCode, WorkError


class ContractValuePolicy:
    SHA256_PATTERN = re.compile(SHA256_PATTERN)

    @staticmethod
    def strict_keys(
        value: object,
        *,
        location: str,
        required: set[str],
        optional: set[str] | None = None,
    ) -> dict[str, Any]:
        if not isinstance(value, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "expected_object",
                "A JSON object is required.",
                {"location": location},
            )
        allowed = required | (optional or set())
        missing = sorted(required - set(value))
        unknown = sorted(set(value) - allowed)
        if missing or unknown:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                {"location": location, "missing": missing, "unknown": unknown},
            )
        return value

    @staticmethod
    def nonempty_string(value: object, *, location: str) -> str:
        if not isinstance(value, str) or not value.strip():
            raise WorkError(
                ExitCode.CONTRACT,
                "empty_text_value",
                "A non-empty string is required.",
                {"location": location},
            )
        return value

    @classmethod
    def sha256(cls, value: object, *, location: str) -> str:
        if not isinstance(value, str) or not cls.SHA256_PATTERN.fullmatch(value):
            raise WorkError(
                ExitCode.CONTRACT,
                INVALID_SHA256_ERROR_CODE,
                "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
                {"location": location},
            )
        return value


__all__ = ["ContractValuePolicy"]
