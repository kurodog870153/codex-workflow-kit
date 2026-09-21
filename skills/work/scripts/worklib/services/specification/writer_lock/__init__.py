"""Specification writer-lock capability."""

from ....technical.infrastructure.writer_lock import require_idle_writer, state_writer

__all__ = ["require_idle_writer", "state_writer"]
