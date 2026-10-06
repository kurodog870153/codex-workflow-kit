//! Public TASK semantic requests.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAcceptanceRemoval {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceTaskReview {
    pub outcome_decisions: String,
    pub technical_decisions: String,
    pub boundary: String,
    pub acceptance: String,
    pub skills: String,
    pub hierarchy: String,
    pub instructions: String,
}

/// The reviewed requirement set and Task impacts bind one exact replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReplacementConfirmation {
    pub previous_planning_sha256: String,
    pub new_source_sha256: String,
    pub requirement_sha256: String,
    pub complete_requirement_review: bool,
    pub retained_acceptance_ids: Vec<String>,
    pub removed_acceptance: Vec<SourceAcceptanceRemoval>,
    pub added_acceptance_ids: Vec<String>,
    pub task_reviews: std::collections::BTreeMap<String, SourceTaskReview>,
}
