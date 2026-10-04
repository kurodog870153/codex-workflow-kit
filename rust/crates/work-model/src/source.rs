//! Read-only current instruction source impact.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImpact {
    pub schema: PublicSchema,
    pub status: String,
    pub changed_sources: u64,
    pub affected_requirements: u64,
    pub affected_files: u64,
    pub blocked: u64,
    pub requirements: Vec<Value>,
}
