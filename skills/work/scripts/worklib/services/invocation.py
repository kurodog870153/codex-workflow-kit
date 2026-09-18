"""Parse explicit Work syntax without interpreting or executing request content."""

from __future__ import annotations

import re

from ..contracts.invocation import InvocationContract
from ..models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import decode_utf8
from ..foundation.paths import validate_requirement_id
from ..protocol import WORKFLOW_MODES


MODES = WORKFLOW_MODES
SYNTAX = "$work <plan|task|execute> -- <request>"


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CLI_USAGE, code, message, {"syntax": SYNTAX, **details})


def parse_invocation(raw: bytes, *, source: str) -> InvocationContract:
    text = decode_utf8(raw, source=source)
    tokens = re.finditer(r"\S+", text)
    first = next(tokens, None)
    if first is None or first.group() != "$work":
        _fail("work_invocation_not_explicit", "An explicit $work invocation must start the input.")
    mode_token = next(tokens, None)
    if mode_token is None or mode_token.group() == "--":
        _fail("work_invocation_mode_missing", "Choose one Work mode.", modes=list(MODES))
    mode = mode_token.group()
    if mode not in MODES:
        _fail("work_invocation_mode_invalid", "Only plan, task and execute are public Work modes.", modes=list(MODES))
    delimiter = next(tokens, None)
    if delimiter is None or delimiter.group() != "--":
        _fail("work_invocation_delimiter", "Place -- directly after the mode, without hierarchy paths or extra tokens.")
    request = text[delimiter.end():]
    if not request.strip():
        _fail("work_invocation_request_missing", "Supply the request after --; surrounding conversation is not a request.")
    entry: dict[str, str] = {"kind": "workflow"}
    words = request.split()
    if mode in {"plan", "task"} and words[0] == "resume":
        if len(words) != 2:
            _fail("work_invocation_resume", "Use exactly resume <requirement-id> for discussion restoration.")
        entry = {"kind": "progress_resume", "requirement_id": validate_requirement_id(words[1])}
    elif mode == "task" and len(words) == 1:
        try:
            requirement = validate_requirement_id(words[0])
        except WorkError:
            pass
        else:
            entry = {"kind": "task_planning", "requirement_id": requirement}
    return InvocationContract(
        schema="work-invocation/v1", mode=mode, request=request, entry=entry,
    )
