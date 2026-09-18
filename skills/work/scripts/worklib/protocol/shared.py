"""Protocol constants with identical meaning across multiple features."""

SHA256_PATTERN = r"^[0-9a-f]{64}$"
WORKFLOW_MODES = ("plan", "task", "execute")


__all__ = ["SHA256_PATTERN", "WORKFLOW_MODES"]
