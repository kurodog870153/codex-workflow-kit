from __future__ import annotations

from enum import IntEnum
from typing import Any


class ExitCode(IntEnum):
    SUCCESS = 0
    CLI_USAGE = 2
    INPUT_FORMAT = 3
    CONTRACT = 4
    ARTIFACT_INTEGRITY = 5
    WORKFLOW_STATE = 6
    LOCK_CONFLICT = 7
    IO_FAILURE = 8
    INTERNAL_ERROR = 10


class WorkError(Exception):
    def __init__(
        self,
        exit_code: ExitCode,
        code: str,
        message: str,
        details: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(message)
        self.exit_code = exit_code
        self.code = code
        self.message = message
        self.details = details or {}

    def as_dict(self) -> dict[str, Any]:
        return {
            "schema": "work-error/v1",
            "code": self.code,
            "message": self.message,
            "details": self.details,
        }
