"""Attempt-close request parsing and validation."""

from pydantic import ValidationError

from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.attempt_close import AttemptCloseRequestContract


def parse_attempt_close_request(raw: bytes, *, source: str) -> AttemptCloseRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, "attempt_close_expected_object", "A JSON object is required.")
    try:
        return AttemptCloseRequestContract.model_validate(value)
    except WorkError:
        raise
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first["loc"])
        if location == ("schema",) and first["type"] not in {"missing", "extra_forbidden"}:
            raise WorkError(ExitCode.CONTRACT, "attempt_close_invalid_schema", "The attempt-close request schema is invalid.") from error
        if location == ("status",) and first["type"] not in {"missing", "extra_forbidden"}:
            raise WorkError(ExitCode.CONTRACT, "attempt_close_invalid_status", "status must be completed, stopped, or blocked.", {"status": value.get("status")}) from error
        field_issues = [item for item in issues if item["type"] in {"missing", "extra_forbidden"}]
        if field_issues:
            raise WorkError(
                ExitCode.CONTRACT, "attempt_close_invalid_fields",
                "The attempt-close request has missing or unknown fields.",
                {
                    "missing": sorted(str(item["loc"][-1]) for item in field_issues if item["type"] == "missing"),
                    "unknown": sorted(str(item["loc"][-1]) for item in field_issues if item["type"] == "extra_forbidden"),
                },
            ) from error
        raise WorkError(ExitCode.CONTRACT, "attempt_close_invalid_fields", "The attempt-close request is invalid.") from error


__all__ = ["parse_attempt_close_request"]
