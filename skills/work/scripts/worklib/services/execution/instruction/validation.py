from __future__ import annotations

from typing import Any

from ....models.common.errors import ExitCode, WorkError


def validate_execute_instruction_selection(
    *, expected_selection: dict[str, Any], current_selection: dict[str, Any],
    expected_sha256: str, operation: str,
) -> dict[str, Any]:
    if (current_selection["selected_paths"] != expected_selection["selected_paths"] or current_selection["resolved_paths"] != expected_selection["resolved_paths"]):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, f"{operation}_execute_instruction_hierarchy_mismatch", "The current Execute hierarchy does not match the target TASK.", None)
    if current_selection["instructions_sha256"] != expected_sha256:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            f"{operation}_execute_instructions_changed",
            "The Execute instruction fingerprint changed after Attempt start.",
            {"expected": expected_sha256, "actual": current_selection["instructions_sha256"]},
        )
    return current_selection
