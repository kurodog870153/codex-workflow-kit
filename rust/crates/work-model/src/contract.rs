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
    #[test]
    fn public_work_examples_use_supported_routes_roles_and_publication_targets() {
        let snapshot = crate::contract_data::registry_value();
        let items = &snapshot["items"];
        assert_eq!(
            items["work-hierarchy/v1"]["description"]["example"]["work_directory"],
            "task"
        );
        assert_eq!(
            items["work-instructions/v1"]["description"]["example"]["hierarchy"]["work_directory"],
            "task"
        );
        assert_eq!(
            items["work-delegation-validation/v1"]["description"]["example"]["role"],
            "task-coordinator"
        );
        let route = &items["work-instruction-selection-manifest/v1"]["description"]["example"]["routing_input"];
        assert_eq!(route["mode"], "task");
        assert_eq!(route["status"], "source_required");
        assert_eq!(route["operation"], "capture_source");
        assert_eq!(
            items["work-operation-envelope/v1"]["description"]["example"]["operation"],
            route["operation"]
        );
        for publication in [
            &items["work-spec-migration-publication/v1"]["description"]["example"],
            &items["work-spec-reconciliation-publication/v1"]["description"]["example"]["publication"],
        ] {
            serde_json::from_value::<crate::specification::SpecMigrationPublication>(
                publication.clone(),
            )
            .unwrap();
            for path in publication["documents"].as_array().unwrap() {
                let path = path.as_str().unwrap();
                assert!(
                    path.starts_with("outputs/work/tasks/")
                        || path.starts_with("outputs/work/executions/")
                );
            }
        }
        assert_eq!(
            items["work-spec-reconciliation-prepare-request/v1"]["scaffold"]["scaffold"]["edits"]
                [0]["target"]["artifact"],
            "task_index"
        );
    }
}
