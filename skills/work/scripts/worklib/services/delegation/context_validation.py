"""Pure validation primitives for delegation context data."""
from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ...foundation.paths import validate_artifact_paths, validate_requirement_id
from .envelope import fail


def nonempty_string(value: Any, *, location: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(location + " must be a nonempty string.")
    return value


def strict_keys(value: Any, *, location: str, required: set[str], optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(location + " must be an object.")
    allowed = required | (optional or set())
    if set(value) != required and not (required <= set(value) <= allowed):
        fail(location + " must contain exactly the required and optional fields.")
    return value


def sha256(value: Any, *, location: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        fail(location + " must be a lowercase SHA-256 digest.")
    return value


def nonempty_object(value: Any, *, location: str) -> dict[str, Any]:
    if not isinstance(value, dict) or not value:
        fail(location + " must be a nonempty object.")
    return value


def text_array(value: Any, *, location: str, task_ids: bool = False) -> None:
    if not isinstance(value, list):
        fail(location + " must be an array.")
    for item in value:
        nonempty_string(item, location=location)
        if task_ids and not re.fullmatch(r"TASK-[0-9]{3}", item):
            fail("Affected TASK IDs must use TASK-nnn.")
    if task_ids and len(value) != len(set(value)):
        fail("Affected TASK IDs must be unique.")


def requirement_id(value: Any) -> str:
    return validate_requirement_id(nonempty_string(value, location="requirement_id"))


def artifact_paths(project_root: Path, requirement: str, paths: Any) -> None:
    validate_artifact_paths(
        project_root,
        requirement,
        paths,
        actual_plan_path=paths.get("plan") if isinstance(paths, dict) else "",
    )
