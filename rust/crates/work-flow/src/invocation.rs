//! Invocation parse command flow.

use serde_json::Value;
use work_feature::error::WorkError;

pub fn parse(raw: &[u8]) -> Result<Value, WorkError> {
    let text = std::str::from_utf8(raw).expect("input reader validated UTF-8");
    work_feature::invocation::parse(text)
}

pub fn confirm(raw: &[u8]) -> Result<Value, WorkError> {
    // Request text is opaque: contract normalization must not alter user confirmation.
    let input: work_model::invocation::ImplicitInvocationRequest = serde_json::from_slice(raw)
        .map_err(|_| {
            WorkError::new(
                work_feature::error::ExitCode::CliUsage,
                "work_invocation_confirmation_invalid",
                "A strict mode, request and user confirmation are required.",
                serde_json::json!({}),
            )
        })?;
    work_feature::invocation::confirm(
        &serde_json::to_value(input).expect("confirmed request serializes"),
    )
}
