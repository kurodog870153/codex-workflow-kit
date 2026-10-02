//! Instruction source and selection data shapes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::schema::PublicSchema;

pub fn verified<T: serde::de::DeserializeOwned>(value: Value) -> Value {
    let _: T = serde_json::from_value(value.clone()).expect("Instruction output matches its model");
    value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionMode {
    Task,
    Execute,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionCatalog {
    pub schema: PublicSchema,
    pub mode: InstructionMode,
    pub paths: Vec<String>,
    pub children: BTreeMap<String, Vec<String>>,
    pub metadata: BTreeMap<String, Value>,
    pub catalog_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPreviewStatus {
    ReviewRequired,
    MigrationRequired,
    Current,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionMigrationPreview {
    pub schema: PublicSchema,
    pub status: MigrationPreviewStatus,
    pub requirement_id: String,
    pub router_compatibility_revision: u64,
    pub affected: BTreeMap<String, u64>,
    pub excluded: Vec<Value>,
    pub files: Vec<Value>,
    pub approved_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPublicationStatus {
    Updated,
    AlreadyCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionMigrationPublication {
    pub schema: PublicSchema,
    pub status: MigrationPublicationStatus,
    pub requirement_id: String,
    pub approved_sha256: String,
    pub transaction_approval_sha256: String,
    pub journal: String,
    pub completion_marker: String,
    pub updated_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionSelectionResponse {
    pub schema: PublicSchema,
    pub mode: InstructionMode,
    pub instruction_selection: InstructionSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionsResponse {
    pub schema: PublicSchema,
    pub mode: InstructionMode,
    pub hierarchy: crate::hierarchy::Hierarchy,
    pub sources: Vec<SourceSummary>,
    pub references: Vec<String>,
    pub instructions_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRoutingInput {
    pub mode: String,
    pub status: String,
    pub operation: String,
    pub artifact_lifecycle: String,
    pub formal_events: Vec<String>,
    pub role: String,
    pub authorization_state: String,
    pub verified_state_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSource {
    pub logical_name: String,
    pub path: String,
    pub compatibility_revision: u64,
    pub canonical_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestRoutingStatus {
    #[serde(rename = "VALID")]
    Valid,
    #[serde(rename = "REVIEW_REQUIRED")]
    ReviewRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionSelectionManifest {
    pub schema: PublicSchema,
    pub router_compatibility_revision: u64,
    pub routing_input: ManifestRoutingInput,
    pub routing_status: ManifestRoutingStatus,
    pub sources: Vec<ManifestSource>,
    pub confirmation_required: bool,
    pub selection_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ModeCatalog {
    pub paths: Vec<String>,
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSummary {
    pub kind: String,
    pub logical_name: String,
    pub canonical_sha256: String,
    pub compatibility_revision: u64,
}

#[derive(Debug, Clone)]
pub struct LoadedSource {
    pub summary: SourceSummary,
    pub canonical_content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionSelection {
    pub selected_paths: Vec<String>,
    pub resolved_paths: Vec<String>,
    pub sources: Vec<SourceSummary>,
    pub references: Vec<String>,
    pub instructions_sha256: String,
}

#[derive(Debug, Clone)]
pub struct SourceSet {
    pub mode: String,
    pub hierarchy: crate::hierarchy::Hierarchy,
    pub sources: Vec<LoadedSource>,
    pub references: Vec<String>,
    pub instructions_sha256: String,
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn public_instruction_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        assert!(
            serde_json::from_value::<InstructionCatalog>(
                items["work-instruction-catalog/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<InstructionMigrationPreview>(
                items["work-instruction-migration-preview/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<InstructionMigrationPublication>(
                items["work-instruction-migration-publication/v1"]["description"]["example"]
                    .clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<InstructionSelectionManifest>(
                items["work-instruction-selection-manifest/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<InstructionSelectionResponse>(
                items["work-instruction-selection/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<InstructionsResponse>(
                items["work-instructions/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
    }
}
