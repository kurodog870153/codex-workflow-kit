//! Pre-execution workflow state assembly.

use serde_json::{Value, json};
use work_operations::canonical::canonical_json_sha256;
use work_operations::operation::{operation_effect, routing_identity};
use work_operations::routing::RoutingRequest;
use work_operations::workflow::{
    WorkflowDecision, decide_execution, decide_pre_execution, lifecycle_for_status,
    mode_for_action, next_action_guidance,
};

use crate::error::{ExitCode, WorkError};

pub trait WorkflowRoutingRepository {
    fn route(&mut self, request: &RoutingRequest<'_>) -> Result<Value, WorkError>;
}

pub struct WorkflowSnapshot {
    pub artifacts: Value,
    pub plan: Option<Value>,
    pub draft: Option<Value>,
    pub task: Option<Value>,
    pub index: Option<Value>,
    pub latest_attempts: Value,
}

pub struct OperationContextRequest<'a> {
    pub command: &'a str,
    pub operation: &'a str,
    pub delegated_role: Option<&'a str>,
    pub artifacts: &'a Value,
    pub project_root: &'a str,
    pub approval_sha256: Option<&'a str>,
    pub transaction_workspace: Option<&'a str>,
}

pub fn build_operation_context(
    routing: &mut impl WorkflowRoutingRepository,
    input: &OperationContextRequest<'_>,
) -> Result<(Value, Value), WorkError> {
    let command = input.command;
    let operation = input.operation;
    let delegated_role = input.delegated_role;
    let artifacts = input.artifacts;
    let project_root = input.project_root;
    let approval_sha256 = input.approval_sha256;
    let transaction_workspace = input.transaction_workspace;
    let effect = operation_effect(command, operation).ok_or_else(|| {
        WorkError::new(
            ExitCode::WorkflowState,
            "operation_effect_unclassified",
            "The command operation has no explicit side-effect classification.",
            json!({"command":command,"operation":operation}),
        )
    })?;
    let identity =
        routing_identity(command, operation, delegated_role).expect("classified operation");
    let state_sha256 = canonical_json_sha256(&json!({
        "command":command,"operation":operation,"mode":identity.mode,
        "events":identity.formal_events,"role":identity.role,"artifacts":artifacts,
        "project_root":project_root,
    }))
    .expect("operation state serializes");
    let request = RoutingRequest {
        status: &format!("cli_{operation}"),
        operation: identity.next_action,
        confirmation: false,
        mode: Some(identity.mode),
        artifact_lifecycle: "verified_cli_input",
        formal_events: &identity.formal_events,
        role: &identity.role,
        authorization_state: Some(effect.authorization_state()),
        verified_state_sha256: &state_sha256,
    };
    let selection = routing.route(&request)?;
    if selection["routing_status"] != "VALID" {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "operation_routing_review_required",
            "The operation context cannot be created from the verified routing state.",
            json!({"routing_reasons":selection["routing_reasons"]}),
        ));
    }
    let mut envelope = json!({
        "schema":"work-operation-envelope/v1","workflow":identity.mode,"operation":operation,
        "verified_state_sha256":state_sha256,"selection_sha256":selection["selection_sha256"],
        "artifacts":artifacts,"approval_sha256":approval_sha256,
        "authorization_state":effect.authorization_state(),
        "side_effect_boundary":effect.side_effect_boundary(),
        "transaction_workspace":transaction_workspace,"role":identity.role,
        "expected_result_contract":"work-operation-result/v1",
    });
    envelope["context_sha256"] =
        json!(canonical_json_sha256(&envelope).expect("operation context serializes"));
    let _: work_model::operation::OperationEnvelope =
        serde_json::from_value(envelope.clone()).expect("bound operation matches its model");
    Ok((envelope, selection))
}

pub fn validate_operation_context(
    envelope: &Value,
    selection: &Value,
    artifacts: &Value,
) -> Result<(), WorkError> {
    let mut context = envelope.clone();
    context
        .as_object_mut()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_operation_envelope",
                "The operation envelope must be an object.",
                json!({}),
            )
        })?
        .remove("context_sha256");
    if envelope["context_sha256"] != canonical_json_sha256(&context).expect("context serializes") {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "operation_context_identity_mismatch",
            "The operation envelope identity does not match its bound fields.",
            json!({}),
        ));
    }
    if envelope["selection_sha256"] != selection["selection_sha256"] {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "operation_selection_drift",
            "The operation selection changed before the worker started.",
            json!({}),
        ));
    }
    if envelope["artifacts"] != *artifacts {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "operation_artifact_drift",
            "A bound operation artifact changed before the worker started.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn pre_execution_state(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    artifacts: &Value,
    plan_validation: Option<&Value>,
    draft: Option<&Value>,
    task_validation: Option<&Value>,
    execution_index_exists: bool,
) -> Result<Option<Value>, WorkError> {
    pre_execution_state_with_events(
        routing,
        WorkflowRoutingContext {
            requirement_id,
            artifacts,
            formal_events: &[],
        },
        plan_validation,
        draft,
        task_validation,
        execution_index_exists,
    )
}

pub struct WorkflowRoutingContext<'a> {
    pub requirement_id: &'a str,
    pub artifacts: &'a Value,
    pub formal_events: &'a [&'a str],
}

pub fn pre_execution_state_with_events(
    routing: &mut impl WorkflowRoutingRepository,
    context: WorkflowRoutingContext<'_>,
    plan_validation: Option<&Value>,
    draft: Option<&Value>,
    task_validation: Option<&Value>,
    execution_index_exists: bool,
) -> Result<Option<Value>, WorkError> {
    let decision = decide_pre_execution(
        plan_validation,
        draft,
        task_validation,
        execution_index_exists,
    )
    .map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            json!({}),
        )
    })?;
    let Some(decision) = decision else {
        return Ok(None);
    };
    render_state(
        routing,
        context.requirement_id,
        context.artifacts,
        decision,
        context.formal_events,
    )
    .map(Some)
}

pub fn execution_state(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    artifacts: &Value,
    index: &Value,
    latest_attempts: &Value,
) -> Result<Value, WorkError> {
    execution_state_with_events(
        routing,
        WorkflowRoutingContext {
            requirement_id,
            artifacts,
            formal_events: &[],
        },
        index,
        latest_attempts,
    )
}

pub fn execution_state_with_events(
    routing: &mut impl WorkflowRoutingRepository,
    context: WorkflowRoutingContext<'_>,
    index: &Value,
    latest_attempts: &Value,
) -> Result<Value, WorkError> {
    render_state(
        routing,
        context.requirement_id,
        context.artifacts,
        decide_execution(index, latest_attempts),
        context.formal_events,
    )
}

fn render_state(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    artifacts: &Value,
    decision: WorkflowDecision,
    formal_events: &[&str],
) -> Result<Value, WorkError> {
    let target = artifacts[decision.target_artifact]
        .as_str()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "workflow_artifact_path_missing",
                "The selected workflow artifact path is missing.",
                json!({"artifact":decision.target_artifact}),
            )
        })?;
    let state_sha256 = canonical_json_sha256(&json!({
        "requirement_id":requirement_id,"status":decision.status,"next_action":decision.next_action,
        "target":target,"artifacts":artifacts,
    }))
    .expect("workflow state serializes");
    let routing_request = RoutingRequest {
        status: &decision.status,
        operation: &decision.next_action,
        confirmation: decision.requires_user_confirmation,
        mode: Some(mode_for_action(&decision.status, &decision.next_action)),
        artifact_lifecycle: lifecycle_for_status(&decision.status),
        formal_events,
        role: "main",
        authorization_state: Some(if decision.requires_user_confirmation {
            "confirmation_required"
        } else {
            "read_only"
        }),
        verified_state_sha256: &state_sha256,
    };
    let routed = routing.route(&routing_request)?;
    let guidance = next_action_guidance(&decision.next_action, requirement_id, artifacts);
    let mut state = json!({
        "schema":"work-workflow-state/v1","requirement_id":requirement_id,
        "status":decision.status,"next_action":decision.next_action,"target":target,
        "requires_user_confirmation":decision.requires_user_confirmation,
        "required_checks":decision.required_checks,"artifacts":artifacts,"details":decision.details,
    });
    for additions in [&routed, &guidance] {
        let object = additions
            .as_object()
            .expect("workflow additions are objects");
        for (key, value) in object {
            state[key] = value.clone();
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedRouting;

    impl WorkflowRoutingRepository for FixedRouting {
        fn route(&mut self, _request: &RoutingRequest<'_>) -> Result<Value, WorkError> {
            Ok(json!({"routing_status":"VALID","selection_sha256":"0".repeat(64)}))
        }
    }

    #[test]
    fn operation_envelope_binds_read_only_context_and_requires_selection_sha() {
        let artifacts = json!({});
        let (envelope, selection) = build_operation_context(
            &mut FixedRouting,
            &OperationContextRequest {
                command: "plan",
                operation: "semantic-prepare",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: None,
                transaction_workspace: None,
            },
        )
        .unwrap();
        assert_eq!(envelope["schema"], "work-operation-envelope/v1");
        assert_eq!(envelope["side_effect_boundary"], "read_only");
        validate_operation_context(&envelope, &selection, &artifacts).unwrap();
        assert_eq!(
            validate_operation_context(&envelope, &selection, &json!({"request":"changed"}))
                .unwrap_err()
                .reason_code,
            "operation_artifact_drift"
        );
        let mut changed_role = envelope.clone();
        changed_role["role"] = json!("worker");
        assert_eq!(
            validate_operation_context(&changed_role, &selection, &artifacts)
                .unwrap_err()
                .reason_code,
            "operation_context_identity_mismatch"
        );
        let mut missing = envelope;
        missing.as_object_mut().unwrap().remove("selection_sha256");
        assert_eq!(
            validate_operation_context(&missing, &selection, &artifacts)
                .unwrap_err()
                .reason_code,
            "operation_context_identity_mismatch"
        );
        let read = build_operation_context(
            &mut FixedRouting,
            &OperationContextRequest {
                command: "progress",
                operation: "read",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: None,
                transaction_workspace: None,
            },
        )
        .unwrap()
        .0;
        let save = build_operation_context(
            &mut FixedRouting,
            &OperationContextRequest {
                command: "progress",
                operation: "save",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: Some(&"a".repeat(64)),
                transaction_workspace: None,
            },
        )
        .unwrap()
        .0;
        assert_ne!(read["context_sha256"], save["context_sha256"]);
        assert_eq!(read["authorization_state"], "read_only");
        assert_eq!(save["authorization_state"], "authorized");
        assert_eq!(save["side_effect_boundary"], "authorized_atomic_write");
        let command_run = build_operation_context(
            &mut FixedRouting,
            &OperationContextRequest {
                command: "execute",
                operation: "command-run",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: Some(&"a".repeat(64)),
                transaction_workspace: None,
            },
        )
        .unwrap()
        .0;
        assert_eq!(
            command_run["side_effect_boundary"],
            "authorized_external_effect"
        );
        let prepare = build_operation_context(
            &mut FixedRouting,
            &OperationContextRequest {
                command: "execute",
                operation: "command-prepare",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: None,
                transaction_workspace: None,
            },
        )
        .unwrap()
        .0;
        let recovery = build_operation_context(
            &mut FixedRouting,
            &OperationContextRequest {
                command: "execute",
                operation: "recover",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: None,
                transaction_workspace: None,
            },
        )
        .unwrap()
        .0;
        assert_ne!(prepare["context_sha256"], recovery["context_sha256"]);
        assert_eq!(
            work_operations::operation::routing_identity("execute", "recover", None)
                .unwrap()
                .formal_events,
            ["recovery"]
        );
    }
}
