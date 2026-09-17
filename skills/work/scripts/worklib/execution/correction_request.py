from __future__ import annotations

from typing import Any

from ..contracts.correction_create_models import CorrectionCreateRequestContract


def parse_correction_create_request(raw: bytes, *, source: str) -> dict[str, Any]:
    return CorrectionCreateRequestContract.parse_request(
        raw, source=source
    ).to_canonical_dict()
