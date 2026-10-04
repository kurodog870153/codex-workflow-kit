//! Application errors define Work exit codes and reason contracts.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    CliUsage = 2,
    InputFormat = 3,
    Contract = 4,
    ArtifactIntegrity = 5,
    WorkflowState = 6,
    LockConflict = 7,
    IoFailure = 8,
    InternalError = 10,
}

#[derive(Debug, Clone)]
pub struct WorkError {
    pub exit_code: ExitCode,
    pub reason_code: String,
    pub message: String,
    pub details: Value,
}

impl WorkError {
    pub fn new(
        exit_code: ExitCode,
        reason_code: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            exit_code,
            reason_code: reason_code.into(),
            message: message.into(),
            details,
        }
    }

    pub fn status(&self) -> &'static str {
        if matches!(
            self.exit_code,
            ExitCode::IoFailure | ExitCode::InternalError
        ) {
            "failed"
        } else {
            "rejected"
        }
    }
}

impl std::fmt::Display for WorkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_and_statuses_match_current_contract() {
        let cases = [
            (ExitCode::CliUsage, 2, "rejected"),
            (ExitCode::InputFormat, 3, "rejected"),
            (ExitCode::Contract, 4, "rejected"),
            (ExitCode::ArtifactIntegrity, 5, "rejected"),
            (ExitCode::WorkflowState, 6, "rejected"),
            (ExitCode::LockConflict, 7, "rejected"),
            (ExitCode::IoFailure, 8, "failed"),
            (ExitCode::InternalError, 10, "failed"),
        ];
        for (code, numeric, status) in cases {
            let details = serde_json::json!({"location": "task.id"});
            let error = WorkError::new(code, "reason", "message", details.clone());
            assert_eq!(code as i32, numeric);
            assert_eq!(error.status(), status);
            assert_eq!(error.exit_code, code);
            assert_eq!(error.reason_code, "reason");
            assert_eq!(error.to_string(), "message");
            assert_eq!(error.details, details);
        }
    }
}
