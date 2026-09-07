from __future__ import annotations

import copy
from typing import Any

from ..foundation.errors import ExitCode, WorkError


def formal_command(task: dict[str, Any], base_record_id: str) -> dict[str, Any]:
    try:
        command = next(
            item for item in task.get("commands", []) if item["id"] == base_record_id
        )
    except StopIteration as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "command_correction_command_not_found",
            "The reserved command is not defined by the target TASK.",
            {"record_id": base_record_id},
        ) from error
    field = "argv" if command.get("mode") == "argv" else "script"
    return {"mode": command.get("mode"), field: copy.deepcopy(command.get(field))}
