from __future__ import annotations

from pathlib import Path
from typing import Any

from ...technical.infrastructure.json_contract import parse_json_contract
from ...technical.foundation.runtime import installed_work_root


def parse_delegation_request(raw: bytes, *, source: str) -> dict[str, Any]:
    return parse_json_contract(raw, source=source)


def delegation_skill_root() -> Path:
    return installed_work_root()
