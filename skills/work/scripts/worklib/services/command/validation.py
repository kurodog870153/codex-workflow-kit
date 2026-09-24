"""Command request parsing and validation."""

from pydantic import ValidationError

from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.command import CommandCorrectionRequestContract, CommandRunRequestContract


def parse_command_correction_request(raw: bytes, *, source: str) -> CommandCorrectionRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, 'command_correction_expected_object', 'A JSON object is required.')
    try:
        request = CommandCorrectionRequestContract.model_validate(value)
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first['loc'])
        if location == ('schema',) and first['type'] not in {'missing', 'extra_forbidden'}:
            raise WorkError(ExitCode.CONTRACT, 'command_correction_invalid_schema', 'The command-correction request schema is invalid.') from error
        field_issues = [issue for issue in issues if issue['type'] in {'missing', 'extra_forbidden'}]
        if field_issues:
            raise WorkError(ExitCode.CONTRACT, 'command_correction_invalid_fields', 'The command-correction request has missing or unknown fields.', {'missing': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'missing')), 'unknown': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'extra_forbidden'))}) from error
        raise WorkError(ExitCode.CONTRACT, 'command_correction_invalid_fields', 'The command-correction request is invalid.') from error
    return request


def parse_command_run_request(raw: bytes, *, source: str) -> CommandRunRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, 'expected_object', 'A JSON object is required.', {'location': 'command_run'})
    try:
        request = CommandRunRequestContract.model_validate(value)
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first['loc'])
        field_issues = [issue for issue in issues if issue['type'] in {'missing', 'extra_forbidden'}]
        if field_issues:
            raise WorkError(ExitCode.CONTRACT, 'invalid_object_fields', 'The JSON object has missing or unknown fields.', {'location': 'command_run', 'missing': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'missing')), 'unknown': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'extra_forbidden'))}) from error
        if location == ('schema',):
            raise WorkError(ExitCode.WORKFLOW_STATE, 'command_run_schema', 'Use work-command-run-request/v1.', {}) from error
        if location == ('timeout_seconds',):
            raise WorkError(ExitCode.WORKFLOW_STATE, 'command_run_timeout', 'timeout_seconds must be an integer from 1 to 3600.', {}) from error
        raise WorkError(ExitCode.WORKFLOW_STATE, 'command_run_request', 'The command-run request is invalid.', {}) from error
    if not 1 <= request.timeout_seconds <= 3600:
        raise WorkError(ExitCode.WORKFLOW_STATE, 'command_run_timeout', 'timeout_seconds must be an integer from 1 to 3600.', {})
    return request


__all__ = ["parse_command_correction_request", "parse_command_run_request"]
