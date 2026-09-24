"""CLI entry-point hints for verified workflow states.

Hints never authorize an operation. A missing command means the next step needs
human selection or inspection before a specific CLI operation can be chosen.
"""

from __future__ import annotations

from ...models.plan import PlanSemanticRequestContract
from ...models.task_draft import TaskSemanticRequestContract
from ...models.specification.reconciliation import SpecificationReconciliationPrepareRequestContract


def next_action_guidance(
    next_action: str, requirement_id: str, artifacts: dict[str, str],
) -> dict[str, object]:
    common = {"user_config_root": "<user-config-root>"}
    task = {"requirement_id": requirement_id, "plan_path": artifacts["plan"], **common}
    execution = {
        "task_path": artifacts["task"], "execution_dir": artifacts["execution"],
        "task_id": "<confirmed-task-id>", **common,
    }
    mapping: dict[str, tuple[str | None, str | None, dict[str, str], str | None]] = {
        "prepare_plan": (
            "plan semantic-prepare", PlanSemanticRequestContract.contract_id,
            {"input_file": "<semantic-input-file>", **common},
            PlanSemanticRequestContract.contract_id,
        ),
        "confirm_task_list": (
            "task semantic-prepare", TaskSemanticRequestContract.contract_id,
            {"input_file": "<semantic-input-file>", **task},
            TaskSemanticRequestContract.contract_id,
        ),
        "choose_task": ("task draft-status", None, {"requirement_id": requirement_id}, None),
        "confirm_start": ("task draft-status", None, {"requirement_id": requirement_id, "task_id": "<confirmed-task-id>"}, None),
        "confirm_resume": ("task draft-status", None, {"requirement_id": requirement_id, "task_id": "<confirmed-task-id>"}, None),
        "confirm_review": ("task draft-status", None, {"requirement_id": requirement_id, "task_id": "<confirmed-task-id>"}, None),
        "assemble_for_review": (None, None, {}, None),
        "select_task_for_execution": ("execute preflight", None, execution, None),
        "continue_execution": (None, None, {}, None),
        "confirm_retry": ("execute preflight", None, execution, None),
        "inspect_recovery": (None, None, {}, None),
        "review_reconciliation": (
            "task reconciliation-prepare", SpecificationReconciliationPrepareRequestContract.contract_id,
            {"input_file": "<semantic-input-file>", **common}, SpecificationReconciliationPrepareRequestContract.contract_id,
        ),
        "review_completion": (None, None, {}, None),
        "review_state": (None, None, {}, None),
    }
    command, request_contract_id, arguments, semantic_input_contract = mapping[next_action]
    return {
        "request_contract_id": request_contract_id,
        "command": command,
        "arguments": arguments,
        "semantic_input_contract": semantic_input_contract,
    }
