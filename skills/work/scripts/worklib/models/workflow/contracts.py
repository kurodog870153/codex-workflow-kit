from __future__ import annotations

from typing import Any, ClassVar, Literal

from pydantic import Field

from ..common.base import WorkContract


class WorkflowStateContract(WorkContract):
    contract_id: ClassVar[str] = "work-workflow-state/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "requirement_id", "status", "next_action", "target",
        "requires_user_confirmation", "required_checks", "artifacts",
        "routing_status", "router_compatibility_revision",
        "required_instruction_sources", "source_order", "selection_sha256",
        "routing_reasons", "selection_manifest", "details",
        "request_contract_id", "command", "arguments", "semantic_input_contract",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-workflow-state/v1", "requirement_id": "example",
        "status": "plan_required", "next_action": "prepare_plan",
        "target": "outputs/work/plans/example.json",
        "requires_user_confirmation": True, "required_checks": [],
        "artifacts": {}, "routing_status": "VALID",
        "router_compatibility_revision": 3,
        "required_instruction_sources": ["work.instruction-loading", "work.workflow.plan"],
        "source_order": ["work.instruction-loading", "work.workflow.plan"],
        "selection_sha256": "0" * 64,
        "routing_reasons": ["next_action:prepare_plan"],
        "selection_manifest": {}, "details": {},
        "request_contract_id": "work-plan-semantic-request/v1",
        "command": "plan semantic-prepare",
        "arguments": {"input_file": "<semantic-input-file>", "user_config_root": "<user-config-root>"},
        "semantic_input_contract": "work-plan-semantic-request/v1",
    }

    schema_: Literal["work-workflow-state/v1"] = Field(alias="schema")
    requirement_id: str
    status: str
    next_action: str
    target: str | None
    requires_user_confirmation: bool
    required_checks: list[str]
    artifacts: dict[str, str]
    routing_status: Literal["VALID", "REVIEW_REQUIRED"]
    router_compatibility_revision: int
    required_instruction_sources: list[str]
    source_order: list[str]
    selection_sha256: str
    routing_reasons: list[str]
    selection_manifest: dict[str, Any]
    details: dict[str, Any]
    request_contract_id: str | None
    command: str | None
    arguments: dict[str, str]
    semantic_input_contract: str | None


class OperationEnvelopeContract(WorkContract):
    contract_id: ClassVar[str] = "work-operation-envelope/v1"
    contract_kind: ClassVar[Literal["generated_request"]] = "generated_request"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "workflow", "operation", "verified_state_sha256",
        "selection_sha256", "artifacts", "approval_sha256",
        "authorization_state", "side_effect_boundary", "transaction_workspace",
        "role", "expected_result_contract", "context_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-operation-envelope/v1", "workflow": "plan",
        "operation": "prepare_plan", "verified_state_sha256": "0" * 64,
        "selection_sha256": "0" * 64, "artifacts": {}, "approval_sha256": None,
        "authorization_state": "read_only", "side_effect_boundary": "read_only",
        "transaction_workspace": None, "role": "main",
        "expected_result_contract": "work-operation-result/v1",
        "context_sha256": "0" * 64,
    }

    schema_: Literal["work-operation-envelope/v1"] = Field(alias="schema")
    workflow: str
    operation: str
    verified_state_sha256: str
    selection_sha256: str
    artifacts: dict[str, dict[str, str]]
    approval_sha256: str | None
    authorization_state: Literal["read_only", "confirmation_required", "authorized"]
    side_effect_boundary: Literal[
        "read_only", "authorized_atomic_write", "authorized_external_effect",
    ]
    transaction_workspace: str | None
    role: str
    expected_result_contract: str
    context_sha256: str


class OperationResultContract(WorkContract):
    contract_id: ClassVar[str] = "work-operation-result/v1"
    contract_kind: ClassVar[Literal["response"]] = "response"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "context_sha256", "status", "result_contract",
        "result_sha256", "evidence",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-operation-result/v1", "context_sha256": "0" * 64,
        "status": "success", "result_contract": "work-plan-prepare/v1",
        "result_sha256": "0" * 64, "evidence": {},
    }

    schema_: Literal["work-operation-result/v1"] = Field(alias="schema")
    context_sha256: str
    status: Literal["success", "blocked", "review_required", "failed", "interrupted"]
    result_contract: str
    result_sha256: str
    evidence: dict[str, Any]


class InstructionSelectionManifestContract(WorkContract):
    contract_id: ClassVar[str] = "work-instruction-selection-manifest/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "router_compatibility_revision", "routing_input",
        "routing_status", "sources", "confirmation_required", "selection_sha256",
    )
    contract_example: ClassVar[dict[str, Any]] = {
        "schema": "work-instruction-selection-manifest/v1",
        "router_compatibility_revision": 3,
        "routing_input": {
            "mode": "plan", "status": "plan_required", "operation": "prepare_plan",
            "artifact_lifecycle": "missing", "formal_events": [], "role": "main",
            "authorization_state": "confirmation_required",
            "verified_state_sha256": "0" * 64,
        },
        "routing_status": "VALID", "sources": [],
        "confirmation_required": True, "selection_sha256": "0" * 64,
    }

    schema_: Literal["work-instruction-selection-manifest/v1"] = Field(alias="schema")
    router_compatibility_revision: int
    routing_input: dict[str, Any]
    routing_status: Literal["VALID", "REVIEW_REQUIRED"]
    sources: list[dict[str, Any]]
    confirmation_required: bool
    selection_sha256: str
