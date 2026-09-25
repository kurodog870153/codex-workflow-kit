//! Artifact builders used by process boundary integration tests.

use serde_json::Value;

pub fn render_plan(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::plan::render_plan_value(value)
}

pub fn render_task_index(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::task::ordering::render_task(
        value,
        work_operations::task::ordering::TaskDocumentKind::Index,
    )
}

pub fn build_initial_execution_index(
    collection: &Value,
    validation: &Value,
) -> Result<Value, String> {
    work_operations::execution::index::build_initial_execution_index(collection, validation)
        .map_err(|issue| issue.reason_code.to_owned())
}

pub fn render_execution_index(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::execution::index::render_execution_index(value)
}

pub fn selection_sha256(mode: &str, skills: &[Value]) -> String {
    work_operations::skill::selection_sha256(mode, skills)
}

pub fn valid_sha256(value: &str) -> bool {
    work_operations::protocol::valid_sha256(value)
}

pub fn validate_contract_example(id: &str, value: &Value) -> Result<(), String> {
    use work_operations::execution::deviation;
    let execution = match id {
        "work-execution-deviation-proposal/v1" => {
            Some(deviation::validate_deviation_proposal(value))
        }
        "work-execution-deviation/v1" => Some(deviation::validate_deviation_artifact(value)),
        "work-execution-deviation-preview/v1" => Some(deviation::validate_deviation_preview(value)),
        "work-execution-deviation-record/v1" => {
            Some(deviation::validate_deviation_record_response(value))
        }
        "work-execution-deviation-semantic-request/v1" => {
            Some(deviation::validate_semantic_deviation_request(value))
        }
        _ => None,
    };
    if let Some(result) = execution {
        return result.map_err(|issue| issue.reason_code.to_owned());
    }
    match id {
        "work-spec-prepare-request/v1" => {
            work_operations::specification::prepare::validate_prepare_request(value)
                .map_err(|issue| issue.reason_code.to_owned())
        }
        "work-spec-verification-request/v1" => {
            work_operations::specification::verification::validate_request(value)
                .map_err(|issue| issue.reason_code.to_owned())
        }
        _ => Err(format!("unknown contract example: {id}")),
    }
}
