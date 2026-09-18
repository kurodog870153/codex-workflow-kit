from __future__ import annotations

from typing import Any

from ...services.task.document import render_ordered_task_contract
from ...services.task.ordering import order_task_contract


def render_task_contract(contract: dict[str, Any]) -> bytes:
    return render_ordered_task_contract(order_task_contract(contract))


__all__ = ["render_task_contract"]
