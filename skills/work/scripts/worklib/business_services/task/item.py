from __future__ import annotations

from typing import Any

from ...services.instruction.history import stored_selection
from ...services.task.document import parse_task_contract
from ...services.task.item_validation import (
    render_task_item_contract as render_item,
    validate_task_item_contract as validate_item,
)
from ...services.task.ordering import order_task_item_contract
from ...services.task.structure import TASK_OPTIONAL, TASK_REQUIRED


def render_task_item_contract(contract: dict[str, Any]) -> bytes:
    return render_item(contract, ordered_contract=order_task_item_contract(contract))


def validate_task_item_contract(raw: bytes, *, source: str, expected_task_id: str,
                                parsed_contract: dict[str, Any] | None = None) -> dict[str, object]:
    contract = parsed_contract if parsed_contract is not None else parse_task_contract(raw, source=source)
    stored_selection(contract["instruction_selection"])
    return validate_item(raw, source=source, expected_task_id=expected_task_id,
        ordered_contract=order_task_item_contract(contract),
        required_fields=TASK_REQUIRED, optional_fields=TASK_OPTIONAL,
        parsed_contract=contract)
