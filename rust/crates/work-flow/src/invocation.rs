//! Invocation parse command flow.

use serde_json::Value;
use work_feature::error::WorkError;

pub fn parse(raw: &[u8]) -> Result<Value, WorkError> {
    let text = std::str::from_utf8(raw).expect("input reader validated UTF-8");
    work_feature::invocation::parse(text)
}
