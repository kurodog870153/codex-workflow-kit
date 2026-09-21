from __future__ import annotations
from pathlib import Path
from typing import Any
from ...models.task_collection import TaskIndexContract
from ...services.instruction.history import stored_selection
from ...services.task.index_validation import render_task_index_contract as render_index, validate_task_index_contract as validate_index
from ...services.task.ordering import order_task_index_contract
from ...services.task.structure import TOP_OPTIONAL, TOP_REQUIRED

def render_task_index_contract(contract: dict[str, Any]) -> bytes:
    checked = TaskIndexContract.model_validate(contract).to_canonical_dict()
    return render_index(checked, ordered_contract=order_task_index_contract(checked))

def validate_task_index_contract(raw: bytes, *, source: str, actual_index_path: str, project_root: Path) -> dict[str, object]:
    contract = TaskIndexContract.parse_json_bytes(raw, source=source).to_canonical_dict()
    stored_selection(contract["instruction_selection"], document=True)
    return validate_index(raw, source=source, actual_index_path=actual_index_path, project_root=project_root,
        ordered_contract=order_task_index_contract(contract), required_fields=TOP_REQUIRED, optional_fields=TOP_OPTIONAL)
