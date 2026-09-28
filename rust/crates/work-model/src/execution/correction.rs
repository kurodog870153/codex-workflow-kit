//! Correction artifact.

use crate::schema::PublicSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Correction {
    pub schema: PublicSchema,
    pub correction_id: String,
    pub created_at: String,
    pub target_attempt_id: String,
    pub task_collection_sha256: String,
    pub task_index_sha256: String,
    pub task_item_sha256: String,
    pub task_instructions_sha256: String,
    pub execute_instructions_sha256: String,
    pub field: String,
    pub correct_value: String,
    pub reason: String,
}
