"""Correction-create request parsing and validation."""

import re

from pydantic import ValidationError

from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.correction import CorrectionCreateRequestContract


def parse_correction_create_request(raw: bytes, *, source: str) -> CorrectionCreateRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, 'correction_create_expected_object', 'A JSON object is required.')
    try:
        request = CorrectionCreateRequestContract.model_validate(value)
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first['loc'])
        if location == ('schema',) and first['type'] not in {'missing', 'extra_forbidden'}:
            raise WorkError(ExitCode.CONTRACT, 'correction_create_invalid_schema', 'The Correction create request schema is invalid.') from error
        if location == ('invalidates_completion',) and first['type'] not in {'missing', 'extra_forbidden'}:
            raise WorkError(ExitCode.CONTRACT, 'correction_create_invalid_invalidation_flag', 'invalidates_completion must be a boolean.') from error
        field_issues = [issue for issue in issues if issue['type'] in {'missing', 'extra_forbidden'}]
        if field_issues:
            raise WorkError(ExitCode.CONTRACT, 'correction_create_invalid_fields', 'The Correction create request has missing or unknown fields.', {'missing': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'missing')), 'unknown': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'extra_forbidden'))}) from error
        field = str(first['loc'][-1])
        if field in {'field', 'correct_value', 'reason'}:
            raise WorkError(ExitCode.CONTRACT, 'correction_create_empty_text', 'Correction text fields must be non-empty strings.', {'field': field}) from error
        raise
    if not re.fullmatch('ATTEMPT-\\d{3}', request.target_attempt_id):
        raise WorkError(ExitCode.CONTRACT, 'correction_create_invalid_attempt_id', 'target_attempt_id must use ATTEMPT-nnn.')
    for field in ('field', 'correct_value', 'reason'):
        if not getattr(request, field).strip():
            raise WorkError(ExitCode.CONTRACT, 'correction_create_empty_text', 'Correction text fields must be non-empty strings.', {'field': field})
    return request


__all__ = ["parse_correction_create_request"]
