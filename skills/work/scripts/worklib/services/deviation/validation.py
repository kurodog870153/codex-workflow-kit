from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.execution.deviation import ExecutionDeviationImpactModel


def deviation_reconciliation_target(proposal: dict[str, Any]) -> str:
    if ExecutionDeviationImpactModel.crosses_semantic_boundary(proposal["impact"]):
        return "plan_and_task"
    return "task_only"


def deviation_is_blocking(proposal: dict[str, Any]) -> bool:
    return deviation_reconciliation_target(proposal) == "plan_and_task"


def _fail(code: str, message: str) -> None:
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, None)


def validate_deviation_action(proposal: dict[str, Any], task: dict[str, Any], record_kind: str) -> None:
    fields = ("steps", "commands", "operations", "validations", "decisions")
    formal_ids = {item["id"] for field in fields for item in task.get(field, [])}
    action = proposal["action"]
    anchor = proposal["anchor_record_id"]
    base_id = anchor.split("#", 1)[0]
    if base_id not in proposal["task_basis"] or any(item not in formal_ids for item in proposal["task_basis"]):
        _fail("deviation_task_basis", "task_basis must contain only formal target TASK IDs and include the anchor base ID.")
    kind = action["kind"]
    if kind == "replace_command" and (record_kind != "command" or action["record_id"] != anchor):
        _fail("deviation_action_anchor", "replace_command must target the reserved command record.")
    if kind == "skip_record" and action["record_id"] != anchor:
        _fail("deviation_action_anchor", "skip_record must target the reserved record.")
    if kind == "adjust_operation" and (record_kind != "operation" or action["operation"]["id"] != base_id):
        _fail("deviation_action_anchor", "adjust_operation must preserve the reserved operation ID.")
    if kind == "add_command" and (action["after_record_id"] != anchor or action["command"]["id"] in formal_ids):
        _fail("deviation_action_identity", "add_command must follow the anchor and use a new formal ID.")
    if kind == "add_validation" and action["validation"]["id"] in formal_ids:
        _fail("deviation_action_identity", "add_validation must use a new formal ID.")
