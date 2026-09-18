"""Pure Work identifier validation models."""

import re

from .errors import ExitCode, WorkError
from .path_segment import PathSegmentPolicy


class IdentifierPolicy:
    REQUIREMENT = re.compile(r"^[a-z0-9._-]+$")
    WORKFLOW = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
    TRANSACTION = re.compile(r"^[0-9]{8}T[0-9]{6}Z-[0-9a-f]{8}$")

    @classmethod
    def requirement_id(cls, value: str) -> str:
        if not isinstance(value, str) or not cls.REQUIREMENT.fullmatch(value):
            raise WorkError(ExitCode.CONTRACT, "invalid_requirement_id", "The requirement ID must use lowercase letters, digits, dots, underscores, or hyphens.", {"requirement_id": value})
        issue = PathSegmentPolicy.issue(value)
        if issue == "windows_device_name":
            raise WorkError(ExitCode.CONTRACT, issue, "The path contains a reserved Windows device name.", {"field": "requirement_id", "segment": value})
        if issue is not None:
            raise WorkError(ExitCode.CONTRACT, issue, "The path contains an empty, current, or parent segment.", {"field": "requirement_id", "segment": value})
        return value

    @classmethod
    def workflow_id(cls, value: str) -> str:
        if not isinstance(value, str) or not cls.WORKFLOW.fullmatch(value):
            raise WorkError(ExitCode.CONTRACT, "invalid_workflow_id", "The workflow ID must use lowercase alphanumeric words separated by hyphens.", {"workflow_id": value})
        return value

    @classmethod
    def transaction_id(cls, value: str) -> str:
        if not isinstance(value, str) or not cls.TRANSACTION.fullmatch(value):
            raise WorkError(ExitCode.CONTRACT, "invalid_transaction_id", "The transaction ID must use UTC YYYYMMDDTHHMMSSZ followed by an eight-character lowercase hexadecimal suffix.", {"transaction_id": value})
        return value


__all__ = ["IdentifierPolicy"]
