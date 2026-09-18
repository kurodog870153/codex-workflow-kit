from __future__ import annotations

import hashlib
from collections.abc import Callable
from pathlib import Path
from typing import Any

from ...foundation.markdown import render_json_contract
from ...infrastructure.progress_storage import publish_progress_revision
from ...models.common.errors import ExitCode, WorkError


def save_progress(project_root: Path, value: object, *, expected_revision: int, approved_sha256: str, preview: Callable[..., dict[str, Any]], read_progress: Callable[[Path, str, str], dict[str, Any]], validate_sha256: Callable[..., str]) -> dict[str, Any]:
    validate_sha256(approved_sha256, location="approved_sha256")
    def checked_preview() -> dict[str, Any]:
        result = preview(project_root, value, expected_revision=expected_revision)
        if result["approved_sha256"] != approved_sha256:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_approval_changed", "The progress content or saved baseline changed after review.")
        return result
    checked = checked_preview()
    progress = checked["progress"]
    directory = f"outputs/work/progress/{progress['requirement_id']}/{progress['mode']}"
    history = f"{directory}/history/{progress['revision']}"
    raw = publish_progress_revision(project_root, directory=directory, history=history, current_path=checked["path"], prepare_raw=lambda: render_json_contract(checked_preview()["progress"]))
    result = read_progress(project_root, progress["requirement_id"], progress["mode"])
    if result["sha256"] != hashlib.sha256(raw).hexdigest():
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_write_mismatch", "The committed progress changed during verification.")
    return {**result, "schema": "work-progress-save/v1", "approved_sha256": approved_sha256}
