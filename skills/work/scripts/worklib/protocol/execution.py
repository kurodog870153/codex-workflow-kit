"""Stable constants for the execution protocol."""

BLOCKING_STOPPED_TYPES = frozenset(
    {"external_operation_failed", "instructions_changed", "specification_defect"}
)


__all__ = ["BLOCKING_STOPPED_TYPES"]
