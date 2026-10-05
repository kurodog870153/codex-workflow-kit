//! Public TASK validation responses.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::Nullable;
use crate::schema::PublicSchema;

use super::projection::TaskCollectionProjection;

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
    pub source_sha256: String,
    pub instructions_sha256: String,
    pub task_instructions_sha256: BTreeMap<String, String>,
    pub task_skill_ids: BTreeMap<String, Nullable<String>>,
    pub hierarchy_selection_sha256: String,
    pub skill_selection_sha256: String,
    pub collection_contract: TaskCollectionProjection,
}
