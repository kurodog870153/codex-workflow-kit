"""Handoff workflow spanning Plan, Instruction, and Task business services."""

from ..business_services.handoff import (
    build_execute_return_handoff as _build_execute_return_handoff,
    build_plan_to_task_handoff as _build_plan_to_task_handoff,
    build_preflight_return_handoff as _build_preflight_return_handoff,
    build_task_to_execute_handoff as _build_task_to_execute_handoff,
    build_task_to_plan_handoff as _build_task_to_plan_handoff,
    run_handoff as _run_handoff,
    verify_plan_to_task_handoff as _verify_plan_to_task_handoff,
    verify_return_handoff as _verify_return_handoff,
    verify_task_to_execute_handoff as _verify_task_to_execute_handoff,
)
from ..business_services.instruction import build_instruction_selection
from ..business_services.plan import validate_plan_contract
from ..business_services.task.io import load_task_collection


class HandoffOperations:
    build_instruction_selection = staticmethod(build_instruction_selection)
    load_task_collection = staticmethod(load_task_collection)
    validate_plan_contract = staticmethod(validate_plan_contract)


def _bind(operation):
    def bound(*args, **kwargs):
        return operation(*args, **kwargs, operations=HandoffOperations)
    return bound


build_execute_return_handoff = _bind(_build_execute_return_handoff)
build_plan_to_task_handoff = _bind(_build_plan_to_task_handoff)
build_preflight_return_handoff = _bind(_build_preflight_return_handoff)
build_task_to_execute_handoff = _bind(_build_task_to_execute_handoff)
build_task_to_plan_handoff = _bind(_build_task_to_plan_handoff)
run_handoff = _bind(_run_handoff)
verify_plan_to_task_handoff = _bind(_verify_plan_to_task_handoff)
verify_return_handoff = _bind(_verify_return_handoff)
verify_task_to_execute_handoff = _bind(_verify_task_to_execute_handoff)

__all__ = [
    "build_execute_return_handoff", "build_plan_to_task_handoff",
    "build_preflight_return_handoff", "build_task_to_execute_handoff",
    "build_task_to_plan_handoff", "run_handoff",
    "verify_plan_to_task_handoff", "verify_return_handoff",
    "verify_task_to_execute_handoff",
]
