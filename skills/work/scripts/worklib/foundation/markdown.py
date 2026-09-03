from __future__ import annotations

import json
import re
from typing import Any

from .errors import ExitCode, WorkError
from .fingerprint import canonical_text, decode_utf8


H1_PATTERN = re.compile(r"^# (.+)$")


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "duplicate_json_key",
                "The JSON contract contains a duplicate key.",
                {"key": key},
            )
        result[key] = value
    return result


def _reject_json_constant(value: str) -> None:
    raise WorkError(
        ExitCode.INPUT_FORMAT,
        "invalid_json_constant",
        "The JSON contract contains a non-standard numeric constant.",
        {"value": value},
    )


def parse_json_contract(raw: bytes, *, source: str) -> dict[str, Any]:
    text = canonical_text(decode_utf8(raw, source=source))
    try:
        contract = json.loads(
            text,
            object_pairs_hook=_strict_object,
            parse_constant=_reject_json_constant,
        )
    except WorkError:
        raise
    except json.JSONDecodeError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_json_contract",
            "The JSON contract is invalid.",
            {"line": error.lineno, "column": error.colno},
        ) from error
    if not isinstance(contract, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
        )
    return contract


def render_markdown_json_contract(title: str, contract: dict[str, Any]) -> bytes:
    payload = json.dumps(
        contract,
        ensure_ascii=False,
        indent=2,
        allow_nan=False,
    )
    return f"# {title}\n\n```json\n{payload}\n```\n".encode("utf-8")


def require_canonical_markdown_json_contract(
    raw: bytes,
    *,
    title: str,
    contract: dict[str, Any],
    source: str,
) -> bytes:
    rendered = render_markdown_json_contract(title, contract)
    if raw != rendered:
        raise WorkError(
            ExitCode.CONTRACT,
            "noncanonical_markdown_contract",
            "The Markdown contract does not match the required canonical rendering.",
            {"source": source},
        )
    return rendered


def parse_markdown_json_contract(raw: bytes, *, source: str) -> tuple[str, dict[str, Any]]:
    text = canonical_text(decode_utf8(raw, source=source))
    lines = text.splitlines()
    if not lines:
        raise WorkError(
            ExitCode.CONTRACT,
            "empty_markdown_document",
            "The Markdown document is empty.",
        )

    h1_lines = [line for line in lines if H1_PATTERN.fullmatch(line)]
    if len(h1_lines) != 1 or not H1_PATTERN.fullmatch(lines[0]):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_markdown_title",
            "The Markdown document must contain exactly one H1 title on the first line.",
        )
    title = H1_PATTERN.fullmatch(lines[0]).group(1)

    contract_index = 1
    if len(lines) > 1 and lines[1] == "":
        contract_index = 2
    openings = [index for index, line in enumerate(lines) if line == "```json"]
    if openings != [contract_index]:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_json_contract_location",
            "The document must contain one JSON fence immediately after the H1 title.",
        )
    try:
        closing_index = lines.index("```", contract_index + 1)
    except ValueError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "unterminated_json_contract",
            "The JSON contract fence is not closed.",
        ) from error

    payload = "\n".join(lines[contract_index + 1 : closing_index])
    try:
        contract = json.loads(
            payload,
            object_pairs_hook=_strict_object,
            parse_constant=_reject_json_constant,
        )
    except WorkError:
        raise
    except json.JSONDecodeError as error:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_json_contract",
            "The fenced JSON contract is invalid.",
            {"line": error.lineno, "column": error.colno},
        ) from error
    if not isinstance(contract, dict):
        raise WorkError(
            ExitCode.CONTRACT,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
        )
    return title, contract
