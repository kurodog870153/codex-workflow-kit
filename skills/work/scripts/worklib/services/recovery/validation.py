"""Recovery request parsing and validation."""
import re
from pydantic import ValidationError
from ...technical.infrastructure.json_contract import parse_json_contract
from ...models.common.errors import ExitCode, WorkError
from ...models.execution.recovery import ExecutionRecoveryPrepareRequestContract, ExecutionRecoveryRequestContract

def parse_execution_recovery_request(raw: bytes, *, source: str) -> ExecutionRecoveryRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, 'execution_recovery_expected_object', 'A JSON object is required.')
    try:
        request = ExecutionRecoveryRequestContract.model_validate(value)
    except WorkError:
        raise
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first['loc'])
        field_issues = [issue for issue in issues if issue['type'] in {'missing', 'extra_forbidden'}]
        if field_issues:
            raise WorkError(ExitCode.CONTRACT, 'execution_recovery_invalid_fields', 'The execution-recovery request has missing or unknown fields.', {'missing': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'missing')), 'unknown': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'extra_forbidden'))}) from error
        codes = {'schema': ('execution_recovery_invalid_schema', 'The execution-recovery request schema is invalid.'), 'transaction': ('execution_recovery_invalid_transaction', 'transaction is not supported by general execution recovery.'), 'attempt_id': ('execution_recovery_invalid_attempt_id', 'attempt_id must use the canonical ATTEMPT-nnn format.'), 'transaction_files': ('execution_recovery_invalid_file_list', 'transaction_files must be an array.')}
        field = str(location[0])
        code, message = codes[field]
        details = {'transaction': value.get('transaction')} if field == 'transaction' else None
        raise WorkError(ExitCode.CONTRACT, code, message, details) from error
    if not re.fullmatch('ATTEMPT-\\d{3}', request.attempt_id):
        raise WorkError(ExitCode.CONTRACT, 'execution_recovery_invalid_attempt_id', 'attempt_id must use the canonical ATTEMPT-nnn format.')
    return request


def parse_execution_recovery_prepare_request(raw: bytes, *, source: str) -> ExecutionRecoveryPrepareRequestContract:
    value = parse_json_contract(raw, source=source)
    if not isinstance(value, dict):
        raise WorkError(ExitCode.CONTRACT, 'expected_object', 'A JSON object is required.', {'location': 'recovery_prepare'})
    try:
        return ExecutionRecoveryPrepareRequestContract.model_validate(value)
    except ValidationError as error:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        first = issues[0]
        location = tuple(first['loc'])
        field_issues = [issue for issue in issues if issue['type'] in {'missing', 'extra_forbidden'}]
        if field_issues:
            raise WorkError(ExitCode.CONTRACT, 'invalid_object_fields', 'The JSON object has missing or unknown fields.', {'location': 'recovery_prepare', 'missing': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'missing')), 'unknown': sorted((str(issue['loc'][-1]) for issue in field_issues if issue['type'] == 'extra_forbidden'))}) from error
        if location == ('schema',):
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, 'recovery_prepare_schema', 'Use work-execution-recovery-prepare-request/v1.') from error
        recovery_value = {**value, 'schema': 'work-execution-recovery-request/v1', 'transaction_files': []}
        ExecutionRecoveryRequestContract.model_validate(recovery_value)
        raise

__all__ = ["parse_execution_recovery_prepare_request", "parse_execution_recovery_request"]
