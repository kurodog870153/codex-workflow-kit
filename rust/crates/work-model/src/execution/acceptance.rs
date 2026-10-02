//! Verified acceptance progress is stored separately from immutable Task criteria.
use super::attempt::ValidationOutcome;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceStatus {
    Pending,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceEvidence {
    pub task_id: String,
    pub attempt_id: String,
    pub validation_id: String,
    pub record_id: String,
    pub outcome: ValidationOutcome,
    pub evidence: String,
    pub task_item_sha256: String,
    pub task_instructions_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceProgress {
    pub id: String,
    pub status: AcceptanceStatus,
    pub evidence: Vec<AcceptanceEvidence>,
}
