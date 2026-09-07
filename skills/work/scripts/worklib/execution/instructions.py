from __future__ import annotations

from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.runtime import installed_work_root
from ..instructions.selection import build_instruction_selection


BASE_EXECUTE_REFERENCES = ["execute.general.execution-records"]
RECOVERY_REFERENCE = "execute.general.execution-recovery"


def _error(
    exit_code: ExitCode,
    code: str,
    message: str,
    **details: object,
) -> None:
    raise WorkError(exit_code, code, message, details or None)


def validate_execute_instructions(
    task: dict[str, Any],
    attempt: dict[str, Any],
    *,
    operation: str,
) -> dict[str, object]:
    selection = task["instruction_selection"]
    references = list(BASE_EXECUTE_REFERENCES)
    if "continued_from" in attempt:
        references.append(RECOVERY_REFERENCE)
    current = build_instruction_selection(
        skill_root=installed_work_root(),
        mode="execute",
        selected_paths=selection["selected_paths"],
        reference_names=references,
    )
    if (
        current["selected_paths"] != selection["selected_paths"]
        or current["resolved_paths"] != selection["resolved_paths"]
    ):
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            f"{operation}_execute_instruction_hierarchy_mismatch",
            "The current Execute hierarchy does not match the target TASK.",
        )
    if current["instructions_sha256"] != attempt["execute_instructions_sha256"]:
        _error(
            ExitCode.ARTIFACT_INTEGRITY,
            f"{operation}_execute_instructions_changed",
            "The Execute instruction fingerprint changed after Attempt start.",
            expected=attempt["execute_instructions_sha256"],
            actual=current["instructions_sha256"],
        )
    return current
