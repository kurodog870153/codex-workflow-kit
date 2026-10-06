//! Public response envelope for Session and formal publication commands.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscussionResultSchema {
    #[serde(rename = "work-discussion-result")]
    Result,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionResult {
    pub schema: DiscussionResultSchema,
    pub command: String,
    pub requirement_id: String,
    pub data: Value,
}
