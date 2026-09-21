from __future__ import annotations

from typing import Any

from ...services.correction.validation import parse_correction_create_request as _parse


def parse_correction_create_request(raw: bytes, *, source: str) -> dict[str, Any]:
    return _parse(raw, source=source).to_canonical_dict()
