//! Public TASK semantic requests.

use serde::{Deserialize, Serialize};

use crate::common::{Nullable, deserialize_required_nullable};

use super::draft::DraftInstructionSelection;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAcceptanceRemoval {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceTaskReview {
    pub outcome_decisions: String,
    pub technical_decisions: String,
    pub boundary: String,
    pub acceptance: String,
    pub skills: String,
    pub hierarchy: String,
    pub instructions: String,
}

/// The reviewed requirement set and Task impacts bind one exact replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReplacementConfirmation {
    pub previous_planning_sha256: String,
    pub new_source_sha256: String,
    pub requirement_sha256: String,
    pub complete_requirement_review: bool,
    pub retained_acceptance_ids: Vec<String>,
    pub removed_acceptance: Vec<SourceAcceptanceRemoval>,
    pub added_acceptance_ids: Vec<String>,
    pub task_reviews: std::collections::BTreeMap<String, SourceTaskReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTaskReference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upsert_position: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTaskUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_task_id: Option<String>,
    pub title: String,
    pub goal: String,
    pub scope: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub dependencies: Vec<SemanticTaskReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_selection: Option<DraftInstructionSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTaskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<super::draft::PlanningSource>,
    pub upsert: Vec<SemanticTaskUpsert>,
    pub remove_task_ids: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub current_task: Nullable<SemanticTaskReference>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub reason: Nullable<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPlanningUpdateInput {
    pub index: super::draft::TaskPlanningIndex,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semantic_request_keeps_required_null_and_rejects_unknown_nested_fields() {
        let source = json!({
            "upsert":[{"title":"Implement","goal":"Deliver","scope":["Source"],
                "skill_id":null,"dependencies":[],
                "instruction_selection":{"selected_paths":[],"references":[]}}],
            "remove_task_ids":[],"current_task":null,"reason":null
        });
        let parsed: SemanticTaskRequest = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(parsed.current_task, Nullable::Null);
        assert_eq!(serde_json::to_value(parsed).unwrap(), source);

        let mut missing = source.clone();
        missing.as_object_mut().unwrap().remove("reason");
        assert!(serde_json::from_value::<SemanticTaskRequest>(missing).is_err());

        let mut unknown = source;
        unknown["upsert"][0]["instruction_selection"]["unexpected"] = json!(true);
        assert!(serde_json::from_value::<SemanticTaskRequest>(unknown).is_err());
    }
}
