//! TASK collection projection used by semantic validation and creation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

use super::index::{
    FormalTaskStatus, TaskArtifactPaths, TaskChangeOperation, TaskDocumentInstructionSelection,
    TaskExecutionDefaults, TaskIndexDecision, TaskReadiness, TaskSourcePlan,
};
use super::item::{
    TaskCommand, TaskDecision, TaskFile, TaskInput, TaskItemInstructionSelection, TaskOperation,
    TaskRisk, TaskStep, TaskTraceability, TaskValidation,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProjectionItem {
    pub id: String,
    pub title: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub instruction_selection: TaskItemInstructionSelection,
    pub traceability: TaskTraceability,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<TaskInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<TaskDecision>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<TaskFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risks: Option<Vec<TaskRisk>>,
    pub steps: Vec<TaskStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<TaskCommand>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations: Option<Vec<TaskOperation>>,
    pub validations: Vec<TaskValidation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProjectionChangeEdit {
    pub operation: TaskChangeOperation,
    pub path: String,
    /// JSON Pointer values are intentionally open content.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub before: Option<Nullable<Value>>,
    /// JSON Pointer values are intentionally open content.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub after: Option<Nullable<Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProjectionChange {
    pub id: String,
    pub spec_id: String,
    pub date: String,
    pub reason: String,
    pub affected_ids: Vec<String>,
    pub edits: Vec<TaskProjectionChangeEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_change_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCollectionProjection {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub spec_id: String,
    pub status: FormalTaskStatus,
    pub title: String,
    pub summary: String,
    pub artifacts: TaskArtifactPaths,
    pub source_plan: TaskSourcePlan,
    pub instruction_selection: TaskDocumentInstructionSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_defaults: Option<TaskExecutionDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<TaskIndexDecision>>,
    pub tasks: Vec<TaskProjectionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<TaskProjectionChange>>,
    pub readiness: TaskReadiness,
}
