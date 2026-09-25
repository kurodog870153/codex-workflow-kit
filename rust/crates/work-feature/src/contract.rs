//! Public contract registry port.

use serde_json::Value;

use crate::error::WorkError;

pub trait ContractRegistry {
    fn list(&self) -> Value;
    fn describe(&self, contract_id: &str) -> Result<Value, WorkError>;
    fn scaffold(&self, contract_id: &str) -> Result<Value, WorkError>;
}
