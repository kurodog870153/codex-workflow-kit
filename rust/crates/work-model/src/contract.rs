//! Public CLI envelope and contract catalog data shapes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{CliStatus, ContractKind, Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliEnvelope {
    pub schema: PublicSchema,
    pub status: CliStatus,
    pub reason_code: String,
    pub message: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkErrorData {
    pub schema: PublicSchema,
    pub code: String,
    pub message: String,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractCatalogEntry {
    pub id: String,
    pub kind: ContractKind,
    pub caller_constructible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractCatalog {
    pub schema: PublicSchema,
    pub contracts: Vec<ContractCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub reference: Nullable<String>,
    pub constraints: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractDescription {
    pub schema: PublicSchema,
    pub id: String,
    pub kind: ContractKind,
    pub caller_constructible: bool,
    pub required: Vec<String>,
    pub optional: Vec<String>,
    pub canonical_order: Vec<String>,
    pub fields: Vec<ContractField>,
    pub example: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractScaffold {
    pub schema: PublicSchema,
    pub id: String,
    pub canonical_order: Vec<String>,
    pub scaffold: Value,
    pub example: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractScaffoldError {
    pub reason_code: String,
    pub message: String,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractRecord {
    pub description: ContractDescription,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scaffold: Option<ContractScaffold>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scaffold_error: Option<ContractScaffoldError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractRegistrySnapshot {
    pub schema: String,
    pub catalog: ContractCatalog,
    pub items: BTreeMap<String, ContractRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_contract_examples_and_registry_match_models() {
        let snapshot: Value = crate::contract_data::registry_value();
        serde_json::from_value::<ContractRegistrySnapshot>(snapshot.clone()).unwrap();
        let items = &snapshot["items"];
        for (id, model) in [
            ("work-cli-result/v1", "cli"),
            ("work-error/v1", "error"),
            ("work-contract-catalog/v1", "catalog"),
            ("work-contract-description/v1", "description"),
            ("work-contract-scaffold/v1", "scaffold"),
        ] {
            let example = items[id]["description"]["example"].clone();
            match model {
                "cli" => {
                    serde_json::from_value::<CliEnvelope>(example).unwrap();
                }
                "error" => {
                    serde_json::from_value::<WorkErrorData>(example).unwrap();
                }
                "catalog" => {
                    serde_json::from_value::<ContractCatalog>(example).unwrap();
                }
                "description" => {
                    serde_json::from_value::<ContractDescription>(example).unwrap();
                }
                "scaffold" => {
                    serde_json::from_value::<ContractScaffold>(example).unwrap();
                }
                _ => unreachable!(),
            }
        }
    }
}
