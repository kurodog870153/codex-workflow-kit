//! Public contract command flows.

use serde_json::Value;
pub use work_feature::contract::ContractRegistry;
use work_feature::error::WorkError;

pub fn list(registry: &impl ContractRegistry) -> Value {
    registry.list()
}

pub fn describe(registry: &impl ContractRegistry, contract_id: &str) -> Result<Value, WorkError> {
    registry.describe(contract_id)
}

pub fn scaffold(registry: &impl ContractRegistry, contract_id: &str) -> Result<Value, WorkError> {
    registry.scaffold(contract_id)
}
