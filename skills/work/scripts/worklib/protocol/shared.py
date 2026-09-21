"""Protocol constants with identical meaning across multiple features."""

SHA256_PATTERN = r"^[0-9a-f]{64}$"
TASK_ID_PATTERN = r"^TASK-\d{3}$"
ATTEMPT_ID_PATTERN = r"^ATTEMPT-\d{3}$"
WORKFLOW_MODES = ("plan", "task", "execute")
INSTRUCTION_SOURCE_KINDS = frozenset({"workflow", "instruction", "reference"})
INVALID_SHA256_ERROR_CODE = "invalid_sha256"


__all__ = [
    "ATTEMPT_ID_PATTERN",
    "INSTRUCTION_SOURCE_KINDS",
    "INVALID_SHA256_ERROR_CODE",
    "SHA256_PATTERN",
    "TASK_ID_PATTERN",
    "WORKFLOW_MODES",
]
