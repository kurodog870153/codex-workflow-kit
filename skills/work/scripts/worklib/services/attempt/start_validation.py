"""Attempt-start request parsing and validation."""

from pydantic import ValidationError

from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.attempt_start import AttemptStartRequestContract


def parse_attempt_start_request(raw: bytes, *, source: str) -> AttemptStartRequestContract:
    value = parse_json_contract(raw, source=source)
    try:
        return AttemptStartRequestContract.model_validate(value)
    except WorkError:
        raise
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first["loc"])
        if location == ("schema",):
            code, message = "attempt_start_invalid_schema", "The Attempt-start request schema is invalid."
        elif location == ("worktree_snapshot_sha256",) and first["type"] not in {"missing", "extra_forbidden"}:
            code, message = "attempt_start_invalid_worktree_snapshot", "A lowercase worktree snapshot SHA-256 is required."
        elif location[-1:] == ("source_attempt_id",):
            code, message = "attempt_start_invalid_source_attempt", "The continuation source Attempt ID is invalid."
        elif location[-1:] == ("record_id",):
            code, message = "attempt_start_invalid_carried_record_id", "A carried record ID is invalid."
        elif location[-1:] == ("evidence",) and first["type"] not in {"missing", "extra_forbidden"}:
            code, message = "attempt_start_empty_text_value", "A non-empty string is required."
        elif location[-1:] == ("carried_records",) and first["type"] not in {"missing", "extra_forbidden"}:
            code, message = "attempt_start_invalid_carried_records", "carried_records must be an array."
        else:
            field_issues = [item for item in issues if item["type"] in {"missing", "extra_forbidden"}]
            if field_issues:
                parent = tuple(field_issues[0]["loc"][:-1])
                related = [item for item in field_issues if tuple(item["loc"][:-1]) == parent]
                raise WorkError(
                    ExitCode.CONTRACT, "attempt_start_invalid_object_fields",
                    "The JSON object has missing or unknown fields.",
                    {
                        "location": "attempt_start_request" if not parent else ".".join(map(str, parent)),
                        "missing": sorted(str(item["loc"][-1]) for item in related if item["type"] == "missing"),
                        "unknown": sorted(str(item["loc"][-1]) for item in related if item["type"] == "extra_forbidden"),
                    },
                ) from error
            code, message = "attempt_start_invalid_object_fields", "The Attempt-start request is invalid."
        raise WorkError(ExitCode.CONTRACT, code, message) from error


__all__ = ["parse_attempt_start_request"]
