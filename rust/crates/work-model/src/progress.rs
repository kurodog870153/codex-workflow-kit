//! Discussion progress artifact and public command formats.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

pub fn verified<T: serde::de::DeserializeOwned>(value: Value) -> Value {
    let _: T = serde_json::from_value(value.clone()).expect("Progress value matches its model");
    value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressMode {
    Plan,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscussionStatus {
    DiscussionOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedDecision {
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionProgress {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub mode: ProgressMode,
    pub revision: u64,
    pub status: DiscussionStatus,
    pub title: String,
    pub request: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub current_task_id: Nullable<String>,
    pub context: BTreeMap<String, Value>,
    pub source_status: Vec<String>,
    pub notes: Vec<String>,
    pub confirmed_decisions: Vec<ConfirmedDecision>,
    pub tentative: Vec<String>,
    pub open_questions: Vec<String>,
    pub next_discussion_point: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressSaveRequest {
    pub schema: PublicSchema,
    pub path: String,
    pub expected_revision: u64,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub previous_sha256: Nullable<String>,
    pub progress: DiscussionProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressResponseStatus {
    Valid,
    Saved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressReview {
    pub schema: PublicSchema,
    pub status: ProgressResponseStatus,
    pub path: String,
    pub expected_revision: u64,
    pub approved_sha256: String,
    pub progress: DiscussionProgress,
    pub source_validation: String,
    pub evidence_trust: String,
    pub formal_readiness: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressRead {
    pub schema: PublicSchema,
    pub status: ProgressResponseStatus,
    pub path: String,
    pub sha256: String,
    pub progress: DiscussionProgress,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressSave {
    pub schema: PublicSchema,
    pub status: ProgressResponseStatus,
    pub path: String,
    pub sha256: String,
    pub progress: DiscussionProgress,
    pub approved_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_progress_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        macro_rules! example {
            ($id:literal, $model:ty) => {
                serde_json::from_value::<$model>(items[$id]["description"]["example"].clone())
                    .unwrap_or_else(|error| panic!("{}: {error}", $id));
            };
        }
        example!("work-discussion-progress/v1", DiscussionProgress);
        example!("work-progress-save-request/v1", ProgressSaveRequest);
        example!("work-progress-preview/v1", ProgressReview);
        example!("work-progress-prepare/v1", ProgressReview);
        example!("work-progress-read/v1", ProgressRead);
        example!("work-progress-save/v1", ProgressSave);
    }
}
