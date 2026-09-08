from __future__ import annotations

import json
from typing import Any

from .errors import ExitCode, WorkError
from .fingerprint import canonical_text, decode_utf8


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


def render_json_contract(contract: dict[str, Any]) -> bytes:
    payload = json.dumps(
        contract,
        ensure_ascii=False,
        indent=2,
        allow_nan=False,
    )
    return canonical_text(payload).encode("utf-8")


def require_canonical_json_contract(
    raw: bytes,
    *,
    contract: dict[str, Any],
    source: str,
) -> bytes:
    rendered = render_json_contract(contract)
    if raw != rendered:
        raise WorkError(
            ExitCode.CONTRACT,
            "noncanonical_json_contract",
            "The JSON contract does not match the required canonical rendering.",
            {"source": source},
        )
    return rendered
