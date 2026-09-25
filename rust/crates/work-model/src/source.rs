//! Instruction source impact and refresh formats.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImpact {
    pub schema: PublicSchema,
    pub status: String,
    pub refreshable: bool,
    pub changed_sources: u64,
    pub affected_requirements: u64,
    pub affected_files: u64,
    pub blocked: u64,
    pub requirements: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRefreshPreview {
    pub schema: PublicSchema,
    pub status: String,
    pub requirement_id: String,
    pub changed_sources: u64,
    pub affected: BTreeMap<String, u64>,
    pub blocked: Vec<Value>,
    pub files: Vec<Value>,
    pub approved_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRefreshPublication {
    pub schema: PublicSchema,
    pub status: String,
    pub requirement_id: String,
    pub approved_sha256: String,
    pub transaction_approval_sha256: String,
    pub journal: String,
    pub completion_marker: String,
    pub updated_files: Vec<String>,
}
