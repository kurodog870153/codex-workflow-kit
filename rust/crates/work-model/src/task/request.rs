//! Public TASK semantic and reviewed repair requests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

use super::draft::{DraftInstructionSelection, SemanticTaskCandidate};
use super::index::{TaskArtifactPaths, TaskIndex};
use super::item::TaskItem;

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
    pub upsert: Vec<SemanticTaskUpsert>,
    pub remove_task_ids: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub current_task: Nullable<SemanticTaskReference>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub reason: Nullable<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRepairStage {
    Format,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRepairDecision {
    pub location: String,
    pub decision: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRepairEdit {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Semantic field payloads have field-specific shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_after: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissingSemanticTask {
    pub task_position: u64,
    pub title: String,
    pub goal: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub selected_paths: Vec<String>,
    pub references: Vec<String>,
    pub candidate: SemanticTaskCandidate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_positions: Option<Vec<u64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRepairPrepareRequest {
    pub schema: PublicSchema,
    pub stage: TaskRepairStage,
    pub requirement_id: String,
    pub decisions: Vec<TaskRepairDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<Vec<TaskRepairEdit>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_task: Option<MissingSemanticTask>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRepairRequest {
    pub schema: PublicSchema,
    pub stage: TaskRepairStage,
    pub requirement_id: String,
    pub artifacts: TaskArtifactPaths,
    pub expected: BTreeMap<String, Nullable<String>>,
    pub decisions: Vec<TaskRepairDecision>,
    pub task_index: TaskIndex,
    pub task_items: BTreeMap<String, TaskItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRepairPrepareInput {
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
