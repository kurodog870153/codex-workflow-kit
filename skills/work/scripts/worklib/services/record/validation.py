"""Record request parsing and validation."""
from pydantic import ValidationError
from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.record import RecordFinishRequestContract

def parse_record_finish_request(raw: bytes, *, source: str) -> RecordFinishRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, 'record_finish_expected_object', 'A JSON object is required.')
    try:
        return RecordFinishRequestContract.model_validate(value)
    except WorkError:
        raise
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first['loc'])
        if location == ('schema',) and first['type'] not in {'missing', 'extra_forbidden'}:
            raise WorkError(ExitCode.CONTRACT, 'record_finish_invalid_schema', 'The record-finish request schema is invalid.') from error
        if location == ('record',) and first['type'] not in {'missing', 'extra_forbidden'}:
            raise WorkError(ExitCode.CONTRACT, 'record_finish_invalid_record', 'record must be a JSON object.') from error
        if location and location[0] == 'modified_files' and (first['type'] not in {'missing', 'extra_forbidden'}):
            code = 'record_finish_invalid_modified_file' if len(location) > 1 else 'record_finish_invalid_modified_files'
            message = 'Every modified file must be a non-empty string.' if len(location) > 1 else 'modified_files must be a non-empty array when present.'
            raise WorkError(ExitCode.CONTRACT, code, message) from error
        field_issues = [issue for issue in issues if issue['type'] in {'missing', 'extra_forbidden'}]
        if field_issues:
            raise WorkError(ExitCode.CONTRACT, 'record_finish_invalid_fields', 'The record-finish request has missing or unknown fields.', {'missing': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'missing')), 'unknown': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'extra_forbidden'))}) from error
        raise WorkError(ExitCode.CONTRACT, 'record_finish_invalid_fields', 'The record-finish request is invalid.') from error

__all__ = ["parse_record_finish_request"]
