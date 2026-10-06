//! Strict public discussion requests.

use super::{DiscussionOperation, DiscussionSession};
use crate::common::Nullable;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscussionRequestSchema {
    #[serde(rename = "work-discussion-request")]
    Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionRequest {
    pub schema: DiscussionRequestSchema,
    pub requirement_id: String,
    pub command: DiscussionCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscussionCommand {
    Init {
        session: Box<DiscussionSession>,
    },
    Read {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_id: Option<String>,
    },
    Status {},
    Update {
        operation: Box<DiscussionOperation>,
    },
    Question {
        operation: Box<DiscussionOperation>,
    },
    Answer {
        operation: Box<DiscussionOperation>,
    },
    History {
        revision: u64,
    },
    Recover {
        session: Nullable<Box<DiscussionSession>>,
        operation: Nullable<Box<DiscussionOperation>>,
    },
    Preview {
        metadata: Value,
    },
    Apply {
        expected_revision: u64,
        session_sha256: String,
        metadata: Value,
        approved_sha256: String,
        publication_evidence: String,
    },
    #[serde(rename = "recover-publication")]
    RecoverPublication {
        expected_revision: u64,
        session_sha256: String,
        metadata: Value,
        approved_sha256: String,
        publication_evidence: String,
    },
}

impl DiscussionCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Init { .. } => "init",
            Self::Read { .. } => "read",
            Self::Status {} => "status",
            Self::Update { .. } => "update",
            Self::Question { .. } => "question",
            Self::Answer { .. } => "answer",
            Self::History { .. } => "history",
            Self::Recover { .. } => "recover",
            Self::Preview { .. } => "preview",
            Self::Apply { .. } => "apply",
            Self::RecoverPublication { .. } => "recover-publication",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn request_has_exact_schema_and_rejects_implicit_or_unknown_commands() {
        let v = json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"read"}});
        let request: DiscussionRequest = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), v);
        for invalid in [
            json!({"schema":"work-discussion-request/v1","requirement_id":"example","command":{"kind":"read"}}),
            json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"read","draft_ref":{}}}),
            json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"save"}}),
        ] {
            assert!(serde_json::from_value::<DiscussionRequest>(invalid).is_err());
        }
    }
}
