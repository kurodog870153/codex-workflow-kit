from ...models.common.errors import ExitCode, WorkError
from ...models.execution.attempt import ATTEMPT_PATTERN


def next_attempt_id(source_attempt_id: str | None) -> str:
    if source_attempt_id is None:
        return "ATTEMPT-001"
    match = ATTEMPT_PATTERN.fullmatch(source_attempt_id)
    assert match is not None
    number = int(match.group(1)) + 1
    if number > 999:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "attempt_start_id_exhausted",
            "No additional three-digit Attempt ID is available.",
            None,
        )
    return f"ATTEMPT-{number:03d}"
