//! Public TASK artifacts and their nested data shapes.

use serde::{Deserialize, Serialize};

use crate::schema::PublicSchema;

pub mod draft;
pub mod index;
pub mod item;
pub mod projection;
pub mod request;
pub mod response;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskFingerprintItem {
    pub id: String,
    pub task_item_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCollectionFingerprint {
    pub schema: PublicSchema,
    pub task_index_sha256: String,
    pub items: Vec<TaskFingerprintItem>,
}
