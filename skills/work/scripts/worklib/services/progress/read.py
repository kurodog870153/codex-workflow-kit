from __future__ import annotations

import hashlib
from collections.abc import Callable
from pathlib import Path
from typing import Any

from ...foundation.markdown import parse_json_contract, render_json_contract
from ...foundation.paths import validate_requirement_id
from ...foundation.spec_update import storage_path
from ...infrastructure.progress_storage import read_progress_bytes
from ...models.common.errors import ExitCode, WorkError


def read_progress(project_root: Path, requirement_id: str, mode: str, *, validate_progress: Callable[[object], dict[str, Any]]) -> dict[str, Any]:
    validate_requirement_id(requirement_id)
    if mode not in ("plan", "task"):
        raise WorkError(ExitCode.CONTRACT, "invalid_progress_mode", "Only Plan and Task discussions can be saved.")
    directory = f"outputs/work/progress/{requirement_id}/{mode}"
    relative = directory + "/progress.json"
    path = storage_path(project_root, relative)
    if not path.exists():
        raise WorkError(ExitCode.WORKFLOW_STATE, "progress_not_saved", "No committed progress exists for this requirement and mode.")
    raw = read_progress_bytes(path)
    progress = validate_progress(parse_json_contract(raw, source=relative))
    if render_json_contract(progress) != raw:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_not_canonical", "Stored progress is not canonical JSON.")
    if progress["requirement_id"] != requirement_id or progress["mode"] != mode:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_identity_mismatch", "Stored progress identifies another requirement or mode.")
    history = storage_path(project_root, f"{directory}/history/{progress['revision']}/progress.json")
    if read_progress_bytes(history) != raw:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "progress_history_mismatch", "Current progress differs from its saved history.")
    return {"schema": "work-progress-read/v1", "status": "saved", "path": relative, "sha256": hashlib.sha256(raw).hexdigest(), "progress": progress}
