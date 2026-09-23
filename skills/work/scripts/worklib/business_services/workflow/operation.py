from __future__ import annotations

import argparse
from collections.abc import Callable
from pathlib import Path
from typing import Any

from ...models.common.errors import ExitCode, WorkError
from ...models.workflow import OperationEnvelopeContract, OperationResultContract
from ...services.workflow import (
    build_raw_state_sha256, build_routing_selection, build_verified_state_sha256,
    resolve_operation_artifact_path,
)


_BOUND_COMMANDS = frozenset({"plan", "task", "execute", "delegation", "progress", "handoff"})
_OPERATION_EFFECTS = {
    "plan": {"create": "write", "prepare": "read_only", "semantic-prepare": "read_only", "validate": "read_only"},
    "task": {
        **{name: "read_only" for name in (
            "diagnose", "draft-assemble", "draft-check", "draft-init-request", "draft-list-prepare",
            "draft-read", "draft-save-request", "draft-status", "migration-preview",
            "reconciliation-preview", "repair-prepare", "repair-validate", "semantic-prepare",
            "spec-prepare", "spec-validate", "spec-verify", "validate",
        )},
        **{name: "write" for name in (
            "create", "draft-create", "draft-init", "draft-list-recover", "draft-list-update",
            "draft-recover", "draft-recover-request", "draft-save", "draft-source-recover",
            "draft-source-update", "migration-apply", "migration-recover", "reconciliation-apply",
            "recover-create", "repair", "repair-recover", "spec-recover", "spec-update",
        )},
    },
    "execute": {
        **{name: "read_only" for name in (
            "command-prepare", "deviation-prepare", "preflight", "recovery-prepare", "worktree",
        )},
        **{name: "write" for name in (
            "attempt-close", "attempt-start", "command-correction", "correction-create",
            "deviation-record", "record-begin", "record-finish", "recover", "recover-attempt-start",
        )},
        "command-run": "external_effect",
    },
    "delegation": {"validate": "read_only"},
    "progress": {"prepare": "read_only", "read": "read_only", "save": "write", "validate": "read_only"},
    "handoff": {name: "read_only" for name in (
        "build-execute-to-plan", "build-execute-to-task", "build-plan-to-task", "build-task-to-execute",
        "build-task-to-plan", "render", "validate", "verify-execute-to-plan",
        "verify-execute-to-task", "verify-plan-to-task", "verify-task-to-execute", "verify-task-to-plan",
    )},
}


def _command_operation(arguments: argparse.Namespace) -> str:
    return next(
        (str(value) for name, value in vars(arguments).items()
         if name.endswith("_command") and value is not None),
        str(arguments.command),
    )


def _routing_identity(arguments: argparse.Namespace) -> tuple[str, str, tuple[str, ...], str]:
    command = str(arguments.command)
    operation = _command_operation(arguments)
    role = "main"
    if command == "plan":
        return "plan", "prepare_plan", (), role
    if command == "task":
        if "migration" in operation:
            return "specification", "review_reconciliation", ("migration",), role
        if "reconciliation" in operation:
            return "specification", "review_reconciliation", ("reconciliation",), role
        if operation.startswith("spec-"):
            return "specification", "review_reconciliation", ("revision",), role
        return "task", "choose_task", (), role
    if command == "progress":
        event = "progress_read" if operation == "read" else "progress_save"
        return "progress", "continue_execution", (event,), role
    if command == "handoff":
        return "execute", "continue_execution", ("handoff",), role
    if command == "delegation":
        return "execute", "continue_execution", ("delegation",), str(arguments.role)
    event_by_operation = {
        "attempt-start": "attempt_start", "recover-attempt-start": "recovery",
        "attempt-close": "attempt_close", "recover": "recovery",
        "recovery-prepare": "recovery", "correction-create": "correction",
        "command-correction": "command_correction", "command-run": "command_execution",
    }
    event = event_by_operation.get(operation)
    next_action = "select_task_for_execution" if operation in {"preflight", "worktree"} else "continue_execution"
    return "execute", next_action, (event,) if event else (), role


def _operation_effect(command: str, operation: str) -> str:
    try:
        return _OPERATION_EFFECTS[command][operation]
    except KeyError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "operation_effect_unclassified",
            "The command operation has no explicit side-effect classification.",
            {"command": command, "operation": operation},
        ) from error


def _artifact_bindings(
    arguments: argparse.Namespace, project_root: Path,
) -> dict[str, dict[str, str]]:
    bindings: dict[str, dict[str, str]] = {}
    for name, value in sorted(vars(arguments).items()):
        if value is None or not isinstance(value, str):
            continue
        if not (name.endswith("_path") or name.endswith("_file") or name.endswith("_dir")):
            continue
        if name in {"input_file", "output_file"}:
            path = Path(value).resolve()
        else:
            path = resolve_operation_artifact_path(project_root, value, field=name)
        state = "missing"
        if path.is_file():
            state = build_raw_state_sha256(path.read_bytes())
        elif path.is_dir():
            state = "directory"
        bindings[name] = {"path": str(path), "raw_sha256": state}
    return bindings


def _context_payload(envelope: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in envelope.items() if key != "context_sha256"}


def build_cli_operation_context(
    arguments: argparse.Namespace, project_root: Path, skill_root: Path,
) -> tuple[dict[str, Any], dict[str, Any]] | None:
    if str(arguments.command) not in _BOUND_COMMANDS:
        return None
    mode, next_action, events, role = _routing_identity(arguments)
    operation = _command_operation(arguments)
    artifacts = _artifact_bindings(arguments, project_root)
    state_payload = {
        "command": arguments.command, "operation": operation, "mode": mode,
        "events": list(events), "role": role, "artifacts": artifacts,
        "project_root": str(project_root),
    }
    state_sha256 = build_verified_state_sha256(state_payload)
    effect = _operation_effect(str(arguments.command), operation)
    authorization_state = "read_only" if effect == "read_only" else "authorized"
    routing = build_routing_selection(
        skill_root, status=f"cli_{operation}", next_action=next_action,
        confirmation=False, mode=mode, artifact_lifecycle="verified_cli_input",
        formal_events=events, role=role, authorization_state=authorization_state,
        verified_state_sha256=state_sha256,
    )
    if routing["routing_status"] != "VALID":
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "operation_routing_review_required",
            "The operation context cannot be created from the verified routing state.",
            {"routing_reasons": routing["routing_reasons"]},
        )
    envelope: dict[str, Any] = {
        "schema": "work-operation-envelope/v1", "workflow": mode,
        "operation": operation, "verified_state_sha256": state_sha256,
        "selection_sha256": routing["selection_sha256"], "artifacts": artifacts,
        "approval_sha256": getattr(arguments, "approved_sha256", None),
        "authorization_state": authorization_state,
        "side_effect_boundary": {
            "read_only": "read_only", "write": "authorized_atomic_write",
            "external_effect": "authorized_external_effect",
        }[effect],
        "transaction_workspace": getattr(arguments, "execution_dir", None),
        "role": role, "expected_result_contract": "work-operation-result/v1",
    }
    envelope["context_sha256"] = build_verified_state_sha256(envelope)
    canonical = OperationEnvelopeContract.model_validate(envelope).to_canonical_dict()
    validate_operation_context(canonical, routing, arguments, project_root=project_root)
    return canonical, routing


def validate_operation_context(
    envelope: dict[str, Any], routing: dict[str, Any], arguments: argparse.Namespace, *,
    project_root: Path,
) -> None:
    validated = OperationEnvelopeContract.model_validate(envelope).to_canonical_dict()
    if build_verified_state_sha256(_context_payload(validated)) != validated["context_sha256"]:
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "operation_context_identity_mismatch",
            "The operation envelope identity does not match its bound fields.",
        )
    if validated["selection_sha256"] != routing["selection_sha256"]:
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "operation_selection_drift",
            "The operation selection changed before the worker started.",
        )
    if validated["artifacts"] != _artifact_bindings(arguments, project_root):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY, "operation_artifact_drift",
            "A bound operation artifact changed before the worker started.",
        )


def execute_with_operation_context(
    arguments: argparse.Namespace, project_root: Path, skill_root: Path,
    worker: Callable[[], dict[str, object]],
) -> dict[str, object]:
    prepared = build_cli_operation_context(arguments, project_root, skill_root)
    if prepared is None:
        return worker()
    envelope, routing = prepared
    validate_operation_context(envelope, routing, arguments, project_root=project_root)
    result = worker()
    result_contract = str(result.get("schema", ""))
    if not result_contract:
        raise WorkError(
            ExitCode.CONTRACT, "operation_result_contract_missing",
            "The worker result does not declare its result contract.",
        )
    structured = OperationResultContract.model_validate({
        "schema": "work-operation-result/v1",
        "context_sha256": envelope["context_sha256"], "status": "success",
        "result_contract": result_contract,
        "result_sha256": build_verified_state_sha256(result),
        "evidence": {"selection_sha256": envelope["selection_sha256"]},
    })
    if structured.context_sha256 != envelope["context_sha256"]:
        raise WorkError(
            ExitCode.WORKFLOW_STATE, "operation_result_context_mismatch",
            "The worker result is not bound to the active operation context.",
        )
    return result


__all__ = [
    "build_cli_operation_context", "execute_with_operation_context",
    "validate_operation_context",
]

