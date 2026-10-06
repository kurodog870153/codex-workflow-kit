//! Workflow status and next-action command entries.

use serde_json::Value;
use work_feature::error::WorkError;
use work_feature::workflow::{
    WorkflowRoutingContext, WorkflowRoutingRepository, WorkflowSnapshot,
    execution_state_with_events, pre_execution_state_with_events,
};

pub use work_feature::workflow::OperationContextRequest;

pub fn build_operation_context(
    routing: &mut impl WorkflowRoutingRepository,
    input: &OperationContextRequest<'_>,
) -> Result<(Value, Value), WorkError> {
    work_feature::workflow::build_operation_context(routing, input)
}

pub fn validate_operation_context(
    envelope: &Value,
    selection: &Value,
    artifacts: &Value,
) -> Result<(), WorkError> {
    work_feature::workflow::validate_operation_context(envelope, selection, artifacts)
}

pub fn status(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    snapshot: WorkflowSnapshot,
) -> Result<Value, WorkError> {
    status_with_events(routing, requirement_id, snapshot, &[])
}

pub fn status_with_events(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    snapshot: WorkflowSnapshot,
    formal_events: &[&str],
) -> Result<Value, WorkError> {
    let WorkflowSnapshot {
        artifacts,
        source,
        discussion,
        task,
        index,
        latest_attempts,
    } = snapshot;
    if let Some(state) = pre_execution_state_with_events(
        routing,
        WorkflowRoutingContext {
            requirement_id,
            artifacts: &artifacts,
            formal_events,
        },
        source.as_ref(),
        discussion.as_ref(),
        task.as_ref(),
        index.is_some(),
    )? {
        return Ok(state);
    }
    execution_state_with_events(
        routing,
        WorkflowRoutingContext {
            requirement_id,
            artifacts: &artifacts,
            formal_events,
        },
        index
            .as_ref()
            .expect("pre-execution state handled missing index"),
        &latest_attempts,
    )
}

pub fn next(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    snapshot: WorkflowSnapshot,
) -> Result<Value, WorkError> {
    status(routing, requirement_id, snapshot)
}

pub fn next_with_events(
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
    snapshot: WorkflowSnapshot,
    formal_events: &[&str],
) -> Result<Value, WorkError> {
    status_with_events(routing, requirement_id, snapshot, formal_events)
}
