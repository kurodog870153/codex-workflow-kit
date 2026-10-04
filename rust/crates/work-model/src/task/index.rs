//! Formal TASK index and revision history shapes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormalTaskStatus {
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskReadinessStatus {
    Passed,
}

pub use super::source::TaskArtifactPaths;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInstructionSource {
    pub kind: String,
    pub logical_name: String,
    pub canonical_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDocumentInstructionSelection {
    pub sources: Vec<TaskInstructionSource>,
    pub references: Vec<String>,
    pub instructions_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_manifest: Option<crate::instruction::InstructionSelectionManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskExecutionDefaults {
    pub working_directory: String,
    pub os: String,
    pub shell: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskItemReference {
    pub id: String,
    pub path: String,
    pub canonical_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIndexDecision {
    pub id: String,
    pub statement: String,
    pub rationale: String,
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskChangeArtifact {
    TaskIndex,
    TaskItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskChangeOperation {
    Add,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskChangeEdit {
    pub artifact: TaskChangeArtifact,
    pub operation: TaskChangeOperation,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// JSON Pointer values may be any JSON value.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub before: Option<Nullable<Value>>,
    /// JSON Pointer values may be any JSON value.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub after: Option<Nullable<Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIndexChange {
    pub id: String,
    pub spec_id: String,
    pub date: String,
    pub reason: String,
    pub affected_ids: Vec<String>,
    pub edits: Vec<TaskChangeEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReadiness {
    pub status: TaskReadinessStatus,
    pub spec_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIndex {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub spec_id: String,
    pub status: FormalTaskStatus,
    pub title: String,
    pub summary: String,
    pub artifacts: TaskArtifactPaths,
    pub source: super::source::TaskProvenance,
    pub hierarchy_selection: crate::hierarchy::HierarchySelection,
    pub skill_selection: crate::skill::SkillSelection,
    pub acceptance_criteria: Vec<super::source::TaskAcceptance>,
    pub instruction_selection: TaskDocumentInstructionSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_defaults: Option<TaskExecutionDefaults>,
    pub tasks: Vec<TaskItemReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<TaskIndexDecision>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<TaskIndexChange>>,
    pub readiness: TaskReadiness,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn change_edit_preserves_explicit_null_pointer_value() {
        let source = json!({"artifact":"task_item","operation":"replace","path":"/skill_id","task_id":"TASK-001","before":null,"after":"skill"});
        let edit: TaskChangeEdit = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(edit.before, Some(Nullable::Null));
        assert_eq!(serde_json::to_value(edit).unwrap(), source);
    }
}
