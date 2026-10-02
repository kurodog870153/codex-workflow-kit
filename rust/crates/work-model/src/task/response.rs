//! Public TASK validation and draft responses.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

use super::draft::{
    DraftInstructionSelection, PlanningSource, PlanningTaskStatus, TaskDraft, TaskPlanningIndex,
};
use super::projection::TaskCollectionProjection;
use super::request::TaskPlanningUpdateInput;

/// Check a validated producer's output against its public model before returning JSON.
pub fn typed_response<T: DeserializeOwned + Serialize>(value: Value) -> Value {
    let typed: T =
        serde_json::from_value(value).expect("validated TASK response matches its model");
    serde_json::to_value(typed).expect("TASK response serializes")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskValidStatus {
    Valid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIndexValidation {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub spec_id: String,
    pub task_ids: Vec<String>,
    pub task_paths: BTreeMap<String, String>,
    pub task_item_sha256: BTreeMap<String, String>,
    pub task_index_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskItemValidation {
    pub schema: PublicSchema,
    pub task_id: String,
    pub dependencies: Vec<String>,
    pub task_item_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPlanningIndexValidation {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub revision: u64,
    pub task_count: usize,
    pub task_order: Vec<String>,
    pub status: TaskValidStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraftValidation {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub task_id: String,
    pub revision: u64,
    pub planning_status: PlanningTaskStatus,
    pub status: TaskValidStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCollectionValidation {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub spec_id: String,
    pub task_ids: Vec<String>,
    pub task_count: usize,
    pub task_index_sha256: String,
    pub task_item_sha256: BTreeMap<String, String>,
    pub task_collection_sha256: String,
    pub source_plan_sha256: String,
    pub instructions_sha256: String,
    pub task_instructions_sha256: BTreeMap<String, String>,
    pub task_skill_ids: BTreeMap<String, Nullable<String>>,
    pub hierarchy_selection_sha256: String,
    pub skill_selection_sha256: String,
    pub collection_contract: TaskCollectionProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraftSourceCheck {
    pub schema: PublicSchema,
    pub status: TaskValidStatus,
    pub requirement_id: String,
    pub task_id: String,
    pub revision: u64,
    pub source: PlanningSource,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub instructions_sha256: String,
    pub instruction_selection: DraftInstructionSelection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DraftPrepareRequest {
    Initial(TaskPlanningIndex),
    Update(TaskPlanningUpdateInput),
    Source(TaskSourcePrepareRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSourcePrepareRequest {
    pub reason: String,
    pub selections: BTreeMap<String, DraftInstructionSelection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraftPrepare {
    pub schema: PublicSchema,
    pub status: TaskPreparedStatus,
    pub request: DraftPrepareRequest,
    pub index: TaskPlanningIndex,
    pub affected_task_ids: Vec<String>,
    pub drafts: BTreeMap<String, TaskDraft>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPreparedStatus {
    Prepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftMirrorStatus {
    NotApplicable,
    Updated,
    Superseded,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftRecoveryStatus {
    Recovered,
    AlreadyCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftDisplayCopyStatus {
    NotApplicable,
    NotUpdated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraftSave {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub revision: u64,
    pub status: TaskSavedStatus,
    pub mirror_status: DraftMirrorStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSavedStatus {
    Saved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraftRecovery {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub revision: u64,
    pub status: DraftRecoveryStatus,
    pub display_copy: DraftDisplayCopyStatus,
}
