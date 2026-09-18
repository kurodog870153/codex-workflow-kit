from __future__ import annotations

import os
from pathlib import Path

from ..models.common.errors import ExitCode, WorkError


def create_plan_exclusively(path: Path, rendered: bytes, *, normalized: str) -> None:
    if path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        )
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as output:
            output.write(rendered)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE, "plan_create_failed",
            "The canonical Plan could not be created.", {"path": normalized},
        ) from error
