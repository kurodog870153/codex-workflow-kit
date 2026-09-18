"""Task document rendering for already ordered values."""

from typing import Any

from ...technical.infrastructure.json_contract import parse_json_contract, render_json_contract


def parse_task_contract(raw: bytes, *, source: str) -> dict[str, Any]:
    return parse_json_contract(raw, source=source)


def render_ordered_task_contract(contract: dict[str, Any]) -> bytes:
    return render_json_contract(contract)


__all__ = ["parse_task_contract", "render_ordered_task_contract"]
