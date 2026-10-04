//! Public workflow state data shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_required_nullable};
use crate::instruction::ManifestRoutingStatus;
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowState {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub status: String,
    pub next_action: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub target: Nullable<String>,
    pub requires_user_confirmation: bool,
    pub required_checks: Vec<String>,
    pub artifacts: BTreeMap<String, String>,
    pub routing_status: ManifestRoutingStatus,
    pub required_instruction_sources: Vec<String>,
    pub source_order: Vec<String>,
    pub selection_sha256: String,
    pub routing_reasons: Vec<String>,
    pub selection_manifest: Value,
    pub details: Value,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub request_contract_id: Nullable<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub command: Nullable<String>,
    pub arguments: BTreeMap<String, String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub semantic_input_contract: Nullable<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_workflow_example_matches_model() {
        let registry: Value = crate::contract_data::registry_value();
        let example = &registry["items"]["work-workflow-state"]["description"]["example"];
        serde_json::from_value::<WorkflowState>(example.clone()).unwrap();
    }
}
