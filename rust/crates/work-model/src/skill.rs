//! Selected Skill format owned by Task planning and shared with Execution.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema::PublicSchema;

pub fn verified<T: serde::de::DeserializeOwned>(value: Value) -> Value {
    let _: T = serde_json::from_value(value.clone()).expect("Skill output matches its model");
    value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillBundleFile {
    pub path: String,
    pub normalization: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillBundle {
    pub schema: PublicSchema,
    pub files: Vec<SkillBundleFile>,
    pub bundle_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalog {
    pub schema: PublicSchema,
    pub skills: Vec<Value>,
    pub unavailable: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSnapshot {
    pub schema: PublicSchema,
    pub skill: Value,
    pub bundle: SkillBundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillValidationStatus {
    Valid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSelectionValidation {
    pub schema: PublicSchema,
    pub status: SkillValidationStatus,
    pub skill_selection: SkillSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSelectionDecision {
    BaseOnly,
    ExternalSkills,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillModeSupport {
    Declared,
    Inferred,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedSkillModeSupport {
    pub task: SkillModeSupport,
    pub execute: SkillModeSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedSkill {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub root: String,
    pub source: String,
    pub description: String,
    pub mode_support: SelectedSkillModeSupport,
    pub allow_implicit_invocation: bool,
    pub dependency_status: String,
    pub summary_sha256: String,
    pub bundle_sha256: String,
    pub recommendation_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSelection {
    pub schema: PublicSchema,
    pub decision: SkillSelectionDecision,
    pub skills: Vec<Value>,
    pub selection_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSelectionChoice {
    pub scope: String,
    pub root: String,
    pub source: String,
    pub recommendation_reason: String,
    pub dependency_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_support: Option<SelectedSkillModeSupport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSelectionRequest {
    pub decision: SkillSelectionDecision,
    pub skills: Vec<SkillSelectionChoice>,
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn public_skill_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        assert!(
            serde_json::from_value::<SkillBundle>(
                items["work-skill-bundle/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<SkillCatalog>(
                items["work-skill-catalog/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<SkillSelectionValidation>(
                items["work-skill-selection-validation/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<SkillSelection>(
                items["work-skill-selection/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<SkillSnapshot>(
                items["work-skill-snapshot/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
    }
}
