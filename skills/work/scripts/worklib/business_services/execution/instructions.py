from __future__ import annotations

from typing import Any

from ...services.execution.instruction.validation import validate_execute_instruction_selection
from ...services.instruction.root import instruction_root


BASE_EXECUTE_REFERENCES = ["execute.general.execution-records"]
RECOVERY_REFERENCE = "execute.general.execution-recovery"


def validate_execute_instructions(
    task: dict[str, Any], attempt: dict[str, Any], *, operation: str, operations,
) -> dict[str, object]:
    selection = task["instruction_selection"]
    references = list(BASE_EXECUTE_REFERENCES)
    if "continued_from" in attempt:
        references.append(RECOVERY_REFERENCE)
    current = operations.build_instruction_selection(
        skill_root=instruction_root(), mode="execute",
        selected_paths=selection["selected_paths"], reference_names=references,
    )
    return validate_execute_instruction_selection(
        expected_selection=selection, current_selection=current,
        expected_sha256=attempt["execute_instructions_sha256"], operation=operation,
    )
