//! Canonical byte and JSON parsing adapter used at the process boundary.

use serde_json::Value;

pub use work_operations::canonical::JsonContractIssue;
pub use work_operations::derivation::fingerprint;

pub fn parse_json_contract(raw: &[u8]) -> Result<Value, JsonContractIssue> {
    work_operations::canonical::parse_json_contract(raw)
}

pub fn decode_utf8(raw: &[u8]) -> Result<&str, std::str::Utf8Error> {
    work_operations::canonical::decode_utf8(raw)
}

pub fn canonical_json(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::canonical::canonical_json(value)
}
