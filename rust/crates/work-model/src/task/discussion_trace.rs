//! Immutable provenance of a formal collection derived from one discussion revision.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionTrace {
    pub session_path: String,
    pub revision: u64,
    pub content_sha256: String,
    pub decision_versions: BTreeMap<String, u64>,
    pub task_decisions: BTreeMap<String, Vec<String>>,
}
