from __future__ import annotations

from ...models.workflow import WorkflowStateContract
from ...services.attempt.validation import (
    render_attempt_json_contract, validate_attempt_file, validate_execution_index,
)
from ...services.workflow import (
    build_routing_selection, build_verified_state_sha256, inspect_requirement_state,
)
from ...models.specification.reconciliation import SpecificationReconciliationLedgerContract


def inspect_workflow_artifacts(project_root, requirement_id, *, plan_path=None):
    artifacts, observed = inspect_requirement_state(
        project_root, requirement_id, plan_path=plan_path,
    )
    raw = observed["execution_index_raw"]
    if raw is not None:
        validate_execution_index(raw, source=str(observed["execution_index_path"]))
    return artifacts, observed


def load_latest_attempts(
    project_root, artifacts: dict[str, str], execution_index: dict[str, object] | None,
) -> dict[str, dict[str, object]]:
    if execution_index is None:
        return {}
    attempts: dict[str, dict[str, object]] = {}
    for row in execution_index["tasks"]:
        attempt_id = row.get("latest_attempt")
        if attempt_id is None:
            continue
        relative = f"{artifacts['execution']}/{row['id']}/{attempt_id}/attempt.json"
        path = project_root / relative
        if not path.is_file():
            continue
        validate_attempt_file(project_root, relative)
        attempts[row["id"]] = render_attempt_json_contract(
            path.read_bytes(), source=str(path), project_root=project_root,
        )
        ledger_path = path.with_name("reconciliation.json")
        if ledger_path.is_file():
            ledger = SpecificationReconciliationLedgerContract.parse_json_bytes(
                ledger_path.read_bytes(), source=str(ledger_path)
            ).to_canonical_dict()
            if ledger["attempt_path"] != relative:
                raise ValueError("Reconciliation ledger does not reference its Attempt.")
            attempts[row["id"]]["_reconciliation_resolved_ids"] = [
                entry["deviation_id"] for entry in ledger["entries"]
            ]
    return attempts


def _result(skill_root, requirement_id: str, artifacts: dict[str, str], status: str,
            next_action: str, target: str | None, *, confirmation: bool,
            checks: list[str], details: dict[str, object] | None = None) -> dict[str, object]:
    mode = "plan" if next_action == "prepare_plan" else (
        "task" if status.startswith("task_") else (
            "repair" if next_action == "inspect_recovery" else (
                "specification" if next_action == "review_reconciliation" else "execute"
            )
        )
    )
    lifecycle = "missing" if status.endswith("required") or status == "plan_required" else status
    state_sha256 = build_verified_state_sha256({
        "requirement_id": requirement_id, "status": status, "next_action": next_action,
        "target": target, "artifacts": artifacts,
    })
    routing = build_routing_selection(
        skill_root, status=status, next_action=next_action, confirmation=confirmation,
        mode=mode, artifact_lifecycle=lifecycle,
        authorization_state="confirmation_required" if confirmation else "read_only",
        verified_state_sha256=state_sha256,
    )
    return WorkflowStateContract.model_validate({
        "schema": "work-workflow-state/v1", "requirement_id": requirement_id,
        "status": status, "next_action": next_action, "target": target,
        "requires_user_confirmation": confirmation, "required_checks": checks,
        "artifacts": artifacts, **routing, "details": details or {},
    }).to_canonical_dict()


def workflow_state(skill_root, requirement_id: str, artifacts: dict[str, str], *,
                   plan_validation: dict[str, object] | None,
                   draft: dict[str, object] | None,
                   task_validation: dict[str, object] | None,
                   execution_index: dict[str, object] | None,
                   latest_attempts: dict[str, dict[str, object]] | None = None) -> dict[str, object]:
    if plan_validation is None:
        return _result(skill_root, requirement_id, artifacts, "plan_required", "prepare_plan",
                       artifacts["plan"], confirmation=True, checks=[])
    if task_validation is None:
        assert draft is not None
        return _result(skill_root, requirement_id, artifacts, f"task_{draft['status']}", str(draft["next_action"]),
                       artifacts["task"], confirmation=bool(draft["requires_user_confirmation"]),
                       checks=list(draft["required_checks"]),
                       details={"plan_sha256": plan_validation["plan_sha256"], "draft": draft})
    if execution_index is None:
        return _result(skill_root, requirement_id, artifacts, "execution_recovery_required", "inspect_recovery",
                       artifacts["execution"], confirmation=True, checks=["task validate"],
                       details={"task_collection_sha256": task_validation["task_collection_sha256"]})
    attempts = latest_attempts or {}
    lock = execution_index.get("lock")
    if lock is not None:
        row = next(
            (item for item in execution_index["tasks"] if item["id"] == lock.get("task_id")),
            None,
        )
        attempt = attempts.get(str(lock.get("task_id")))
        active_lock = (
            isinstance(lock, dict)
            and lock.get("kind") == "execution"
            and isinstance(row, dict)
            and row.get("status") == "in_progress"
            and row.get("latest_attempt") == lock.get("attempt_id")
            and isinstance(attempt, dict)
            and attempt.get("status") == "in_progress"
            and attempt.get("task_id") == lock.get("task_id")
            and attempt.get("attempt_id") == lock.get("attempt_id")
            and attempt.get("execute_instructions_sha256")
            == lock.get("execute_instructions_sha256")
        )
        if not active_lock:
            return _result(skill_root, requirement_id, artifacts, "execution_locked", "inspect_recovery",
                           artifacts["execution"], confirmation=True, checks=["execute preflight"],
                           details={"lock": lock})
    overall = execution_index["overall_status"]
    unresolved = [
        deviation["deviation_id"]
        for attempt in attempts.values()
        if attempt.get("status") != "in_progress"
        for deviation in attempt.get("execution_deviations", [])
        if deviation["decision"]["outcome"] == "approved"
        and deviation["reconciliation_status"] == "pending"
        and deviation["deviation_id"]
        not in set(attempt.get("_reconciliation_resolved_ids", []))
    ]
    if overall in {"completed", "cancelled"} and unresolved:
        return _result(
            skill_root, requirement_id, artifacts, "execution_reconciliation_pending",
            "review_reconciliation", artifacts["execution"], confirmation=True,
            checks=["task validate", "execute preflight"],
            details={"overall_status": overall, "pending_deviation_ids": sorted(unresolved)},
        )
    action, confirmation = {
        "pending": ("select_task_for_execution", True),
        "in_progress": ("continue_execution", False),
        "pending_retry": ("confirm_retry", True),
        "blocked": ("review_reconciliation", True),
        "completed": ("review_completion", True),
        "cancelled": ("review_completion", True),
    }.get(overall, ("review_state", True))
    return _result(skill_root, requirement_id, artifacts, f"execution_{overall}", action,
                   artifacts["execution"], confirmation=confirmation,
                   checks=["task validate", "execute preflight"], details={"overall_status": overall})
