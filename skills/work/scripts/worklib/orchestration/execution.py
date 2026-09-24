"""Execution workflow spanning Task and Instruction business services."""

from ..business_services.execution.context import load_lifecycle_task_context as _load_context
from ..business_services.execution.instructions import validate_execute_instructions as _validate_instructions
from ..business_services.execution.preflight import execute_preflight as _execute_preflight
from ..business_services.execution.worktree import inspect_execute_worktree as _inspect_execute_worktree
from ..business_services.execution.record_begin import begin_record as _begin_record
from ..business_services.execution.record_finish import finish_record as _finish_record
from ..business_services.execution.attempt_start import recover_attempt_start as _recover_attempt_start, start_attempt as _start_attempt, prepare_attempt_start as _prepare_attempt_start
from ..business_services.execution.attempt_close import close_attempt as _close_attempt
from ..business_services.execution.recovery import recover_execution as _recover_execution
from ..business_services.execution.recovery_prepare import prepare_execution_recovery as _prepare_execution_recovery
from ..business_services.execution.correction import create_correction as _create_correction
from ..business_services.execution.command_correction import record_command_correction as _record_command_correction
from ..business_services.execution.command_run import prepare_command as _prepare_command, run_command as _run_command
from ..business_services.execution.deviation import prepare_execution_deviation as _prepare_execution_deviation, prepare_semantic_execution_deviation as _prepare_semantic_execution_deviation, record_execution_deviation as _record_execution_deviation
from ..business_services.instruction import build_instruction_selection
from ..business_services.task.io import load_task_execution_context, recheck_task_execution_context
from ..business_services.execution.workflow import ExecutionService


class ExecutionOperations:
    build_instruction_selection = staticmethod(build_instruction_selection)
    load_task_execution_context = staticmethod(load_task_execution_context)
    recheck_task_execution_context = staticmethod(recheck_task_execution_context)


class ExecutionCapabilities(ExecutionOperations):
    pass


def load_lifecycle_task_context(*args, **kwargs):
    return _load_context(*args, **kwargs, operations=ExecutionOperations)


def validate_execute_instructions(*args, **kwargs):
    return _validate_instructions(*args, **kwargs, operations=ExecutionOperations)


def execute_preflight(*args, **kwargs):
    return _execute_preflight(*args, **kwargs, operations=ExecutionOperations)


def inspect_execute_worktree(*args, **kwargs):
    return _inspect_execute_worktree(*args, **kwargs, operations=ExecutionOperations)


def begin_record(*args, **kwargs):
    return _begin_record(*args, **kwargs, operations=ExecutionOperations)


def finish_record(*args, **kwargs):
    return _finish_record(*args, **kwargs, operations=ExecutionOperations)


def start_attempt(*args, **kwargs):
    return _start_attempt(*args, **kwargs, operations=ExecutionOperations)


def prepare_attempt_start(*args, **kwargs):
    return _prepare_attempt_start(*args, **kwargs, operations=ExecutionOperations)


def recover_attempt_start(*args, **kwargs):
    return _recover_attempt_start(*args, **kwargs, operations=ExecutionOperations)


def close_attempt(*args, **kwargs):
    return _close_attempt(*args, **kwargs, operations=ExecutionOperations)


def recover_execution(*args, **kwargs):
    return _recover_execution(*args, **kwargs, operations=ExecutionOperations)


def prepare_execution_recovery(*args, **kwargs):
    return _prepare_execution_recovery(*args, **kwargs, operations=ExecutionOperations)


def create_correction(*args, **kwargs):
    return _create_correction(*args, **kwargs, operations=ExecutionOperations)


def record_command_correction(*args, **kwargs):
    return _record_command_correction(*args, **kwargs, operations=ExecutionOperations)


def prepare_command(*args, **kwargs):
    return _prepare_command(*args, **kwargs, operations=ExecutionOperations)


def run_command(*args, **kwargs):
    return _run_command(*args, **kwargs, operations=ExecutionOperations)


def prepare_execution_deviation(*args, **kwargs):
    return _prepare_execution_deviation(*args, **kwargs, operations=ExecutionOperations)


def prepare_semantic_execution_deviation(*args, **kwargs):
    return _prepare_semantic_execution_deviation(*args, **kwargs, operations=ExecutionOperations)


def record_execution_deviation(*args, **kwargs):
    return _record_execution_deviation(*args, **kwargs, operations=ExecutionOperations)


for _name in (
    "begin_record", "close_attempt", "create_correction", "execute_preflight",
    "finish_record", "inspect_execute_worktree", "prepare_command",
    "prepare_execution_deviation", "prepare_execution_recovery",
    "prepare_semantic_execution_deviation",
    "record_command_correction", "record_execution_deviation",
    "prepare_attempt_start", "recover_attempt_start", "recover_execution", "run_command", "start_attempt",
):
    setattr(ExecutionCapabilities, _name, staticmethod(globals()[_name]))


execution_service = ExecutionService(ExecutionCapabilities)


__all__ = [
    "ExecutionCapabilities",
    "ExecutionOperations",
    "begin_record",
    "close_attempt",
    "create_correction",
    "execute_preflight",
    "execution_service",
    "finish_record",
    "inspect_execute_worktree",
    "load_lifecycle_task_context",
    "prepare_command",
    "prepare_attempt_start",
    "prepare_execution_deviation",
    "prepare_semantic_execution_deviation",
    "prepare_execution_recovery",
    "record_command_correction",
    "record_execution_deviation",
    "recover_attempt_start",
    "recover_execution",
    "run_command",
    "start_attempt",
    "validate_execute_instructions",
]
