"""Task repair workflow spanning Task and Specification business services."""

from ..business_services.specification import (
    execution_history_fingerprints,
    rebuild_execution_index,
)
from ..business_services.task.repair import (
    prepare_task_repair as prepare_task_repair_business,
    repair_task as repair_task_business,
)


def repair_task(*args, **kwargs):
    return repair_task_business(
        *args,
        **kwargs,
        execution_history_fingerprints=execution_history_fingerprints,
        rebuild_execution_index=rebuild_execution_index,
    )


def prepare_task_repair(*args, **kwargs):
    return prepare_task_repair_business(
        *args,
        **kwargs,
        execution_history_fingerprints=execution_history_fingerprints,
        rebuild_execution_index=rebuild_execution_index,
    )


__all__ = ["prepare_task_repair", "repair_task"]
