//! Invocation request parsing.

use serde_json::Value;
use work_operations::invocation::parse_invocation;

use crate::error::{ExitCode, WorkError};

pub fn parse(text: &str) -> Result<Value, WorkError> {
    parse_invocation(text).map_err(|issue| {
        WorkError::new(
            if issue.contract {
                ExitCode::Contract
            } else {
                ExitCode::CliUsage
            },
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })
}

pub fn confirm(value: &Value) -> Result<Value, WorkError> {
    work_operations::invocation::confirm_invocation(value).map_err(|issue| {
        WorkError::new(
            ExitCode::CliUsage,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })
}
