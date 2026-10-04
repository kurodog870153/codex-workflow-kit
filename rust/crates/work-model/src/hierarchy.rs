//! Work hierarchy data shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::schema::PublicSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HierarchySelectionDecision {
    InstructionPaths,
    GeneralOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HierarchyChoice {
    pub path: String,
    pub recommendation_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HierarchySelectionRequest {
    pub decision: HierarchySelectionDecision,
    pub selections: Vec<HierarchyChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HierarchySelection {
    pub schema: PublicSchema,
    pub decision: HierarchySelectionDecision,
    pub selected_paths: Vec<String>,
    pub entries: Vec<Value>,
    pub catalog_sha256: String,
    pub selection_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HierarchySelectionValidation {
    pub schema: PublicSchema,
    pub status: HierarchyValidationStatus,
    pub hierarchy_selection: HierarchySelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HierarchyValidationStatus {
    Valid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Hierarchy {
    pub schema: String,
    pub work_directory: String,
    pub selected_paths: Vec<String>,
    pub resolved_paths: Vec<String>,
    pub required_paths: Vec<String>,
    pub optional_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CrossModeCatalog {
    pub paths: Vec<String>,
    pub children: BTreeMap<String, Vec<String>>,
    pub metadata: BTreeMap<String, Value>,
    pub catalog_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_hierarchy_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        assert!(
            serde_json::from_value::<Hierarchy>(
                items["work-hierarchy"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<HierarchySelection>(
                items["work-hierarchy-selection"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<HierarchySelectionValidation>(
                items["work-hierarchy-selection-validation"]["description"]["example"].clone()
            )
            .is_ok()
        );
    }
}
