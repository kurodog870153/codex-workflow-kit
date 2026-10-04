//! Attempt and Correction document operations.

use serde::{Serialize, Serializer};
use serde_json::{Value, json};

pub struct OrderedAttempt<'a>(pub &'a Value);

impl Serialize for OrderedAttempt<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        work_operations::execution::attempt::ordered_attempt(self.0).serialize(serializer)
    }
}

pub struct OrderedCorrection<'a>(pub &'a Value);

impl Serialize for OrderedCorrection<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        work_operations::execution::correction::ordered_correction(self.0).serialize(serializer)
    }
}
use work_operations::execution::ExecutionIssue;
use work_operations::execution::attempt::{
    render_attempt, validate_attempt_bytes, validate_attempt_file_path, validate_attempt_identity,
};
use work_operations::execution::correction::{
    render_correction, validate_correction, validate_correction_file_path,
};

use crate::error::{ExitCode, WorkError};

fn issue(error: ExecutionIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        error.reason_code,
        error.message,
        error.details,
    )
}

pub fn render_attempt_document(value: Value) -> Result<Value, WorkError> {
    render_attempt(&value).map_err(issue)?;
    Ok(value)
}

pub fn validate_attempt_document(
    value: &Value,
    raw: Option<&[u8]>,
    path: Option<&str>,
) -> Result<Value, WorkError> {
    let mut result = if let Some(raw) = raw {
        validate_attempt_bytes(value, raw).map_err(issue)?
    } else {
        validate_attempt_identity(value).map_err(issue)?;
        json!({"schema":"work-attempt-validation","attempt_id":value["attempt_id"],
            "task_spec_id":value["task_spec_id"],"task_id":value["task_id"],
            "status":value["status"],"record_count":value["records"].as_array().map_or(0,Vec::len),
            "result":"valid"})
    };
    if let Some(path) = path {
        validate_attempt_file_path(path, value).map_err(issue)?;
        result["path"] = json!(path);
    }
    Ok(result)
}

pub fn render_correction_document(value: Value) -> Result<Value, WorkError> {
    render_correction(&value).map_err(issue)?;
    Ok(value)
}

pub fn validate_correction_document(value: &Value, path: Option<&str>) -> Result<Value, WorkError> {
    let mut result = validate_correction(value).map_err(issue)?;
    if let Some(path) = path {
        validate_correction_file_path(path, value).map_err(issue)?;
        result["path"] = json!(path);
    }
    Ok(result)
}
