//! Work Plan data shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::common::{Nullable, deserialize_optional_nullable};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanGoal {
    pub id: String,
    pub statement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    InScope,
    OutOfScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanScope {
    pub id: String,
    pub kind: ScopeKind,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanAppliesTo {
    pub id: String,
    pub statement: String,
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRisk {
    pub id: String,
    pub condition: String,
    pub impact: String,
    pub mitigation: String,
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanMilestone {
    pub id: String,
    pub statement: String,
    pub deliverable_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDeliverable {
    pub id: String,
    pub statement: String,
    pub goal_ids: Vec<String>,
    pub acceptance_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanAcceptance {
    pub id: String,
    pub statement: String,
    pub deliverable_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDecision {
    pub id: String,
    pub statement: String,
    pub rationale: String,
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanChange {
    pub id: String,
    pub date: String,
    pub location: String,
    pub before: String,
    pub after: String,
    pub reason: String,
    pub affected_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Confirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanArtifact {
    pub schema: String,
    pub requirement_id: String,
    pub status: PlanStatus,
    pub title: String,
    pub summary: String,
    pub artifacts: BTreeMap<String, Value>,
    pub hierarchy_selection: BTreeMap<String, Value>,
    pub work_instruction_selection: BTreeMap<String, Value>,
    pub skill_selection: BTreeMap<String, Value>,
    pub goals: Vec<PlanGoal>,
    pub scope: Vec<PlanScope>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub constraints: Option<Nullable<Vec<PlanAppliesTo>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub dependencies: Option<Nullable<Vec<PlanAppliesTo>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub risks: Option<Nullable<Vec<PlanRisk>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub milestones: Option<Nullable<Vec<PlanMilestone>>>,
    pub deliverables: Vec<PlanDeliverable>,
    pub acceptance_criteria: Vec<PlanAcceptance>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub decisions: Option<Nullable<Vec<PlanDecision>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub changes: Option<Nullable<Vec<PlanChange>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSemanticRequest {
    pub requirement_id: String,
    pub title: String,
    pub summary: String,
    pub goals: Vec<String>,
    pub scope: Vec<String>,
    pub deliverables: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub hierarchy_selection_request: crate::hierarchy::HierarchySelectionRequest,
    pub skill_selection_request: crate::skill::SkillSelectionRequest,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanValidation {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub status: PlanStatus,
    pub plan_sha256: String,
    pub hierarchy_selection_sha256: String,
    pub work_instructions_sha256: String,
    pub skill_selection_sha256: String,
    pub item_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanPrepare {
    pub schema: PublicSchema,
    pub status: PrepareStatus,
    pub path: String,
    pub plan: PlanArtifact,
    pub validation: PlanValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrepareStatus {
    Prepared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanCreate {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub status: PlanStatus,
    pub plan_sha256: String,
    pub hierarchy_selection_sha256: String,
    pub work_instructions_sha256: String,
    pub skill_selection_sha256: String,
    pub item_count: u64,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_plan_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        assert!(
            serde_json::from_value::<PlanArtifact>(
                items["work-plan/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PlanSemanticRequest>(
                items["work-plan-semantic-request/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PlanValidation>(
                items["work-plan-validation/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PlanPrepare>(
                items["work-plan-prepare/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PlanCreate>(
                items["work-plan-create/v1"]["description"]["example"].clone()
            )
            .is_ok()
        );
    }
}
