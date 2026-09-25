//! Bound operation envelope and result formats.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationEnvelope {
    pub schema: PublicSchema,
    pub workflow: String,
    pub operation: String,
    pub verified_state_sha256: String,
    pub selection_sha256: String,
    pub artifacts: BTreeMap<String, Value>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub approval_sha256: Nullable<String>,
    pub authorization_state: String,
    pub side_effect_boundary: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub transaction_workspace: Nullable<String>,
    pub role: String,
    pub expected_result_contract: String,
    pub context_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationResult {
    pub schema: PublicSchema,
    pub context_sha256: String,
    pub status: String,
    pub result_contract: String,
    pub result_sha256: String,
    pub evidence: Value,
}
