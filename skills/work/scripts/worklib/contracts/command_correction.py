from __future__ import annotations

from typing import Any

from ..foundation.errors import ExitCode, WorkError


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, details or None)


def _strict_object(
    value: object,
    *,
    location: str,
    required: set[str],
) -> dict[str, Any]:
    if not isinstance(value, dict):
        _fail("attempt_expected_object", "A JSON object is required.", location=location)
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - required)
    if missing or unknown:
        _fail(
            "attempt_invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            location=location,
            missing=missing,
            unknown=unknown,
        )
    return value


def _nonempty(value: object, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        _fail(
            "attempt_empty_text_value",
            "A non-empty string is required.",
            location=location,
        )
    return value


def _command_value(value: object, *, location: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        _fail("attempt_expected_object", "A JSON object is required.", location=location)
    mode = value.get("mode")
    if mode == "argv":
        command = _strict_object(
            value,
            location=location,
            required={"mode", "argv"},
        )
        argv = command["argv"]
        if not isinstance(argv, list) or not argv:
            _fail(
                "attempt_invalid_command_argv",
                "A command argv must be a non-empty string array.",
                location=f"{location}.argv",
            )
        canonical_argv = [
            _nonempty(item, location=f"{location}.argv[]") for item in argv
        ]
        return {"mode": "argv", "argv": canonical_argv}
    if mode == "shell":
        command = _strict_object(
            value,
            location=location,
            required={"mode", "script"},
        )
        return {
            "mode": "shell",
            "script": _nonempty(command["script"], location=f"{location}.script"),
        }
    _fail(
        "attempt_invalid_command_mode",
        "A command mode must be argv or shell.",
        location=f"{location}.mode",
    )


def canonicalize_command_correction(
    value: object, *, location: str = "correction"
) -> dict[str, Any]:
    correction = _strict_object(
        value,
        location=location,
        required={
            "original_command",
            "actual_command",
            "reason",
            "authorization_evidence",
        },
    )
    original = _command_value(
        correction["original_command"], location=f"{location}.original_command"
    )
    actual = _command_value(
        correction["actual_command"], location=f"{location}.actual_command"
    )
    if original["mode"] != actual["mode"]:
        _fail(
            "attempt_command_correction_mode_mismatch",
            "An equivalent command correction must preserve the command mode.",
            location=location,
        )
    if original == actual:
        _fail(
            "attempt_command_correction_unchanged",
            "A command correction must change the command value.",
            location=location,
        )
    return {
        "original_command": original,
        "actual_command": actual,
        "reason": _nonempty(correction["reason"], location=f"{location}.reason"),
        "authorization_evidence": _nonempty(
            correction["authorization_evidence"],
            location=f"{location}.authorization_evidence",
        ),
    }
