//! Requirement-owned discussion models; public registration is a separate boundary.

use std::collections::BTreeMap;

pub mod contracts;
pub mod operation;
pub mod request;
pub mod response;
pub use operation::{DiscussionChange, DiscussionOperation};

use serde::{Deserialize, Serialize};

use crate::common::{Nullable, deserialize_required_nullable};
use crate::task::candidate::{
    CandidateCommand, CandidateFile, CandidateInput, CandidateOperation, CandidateRisk,
    CandidateStep, CandidateValidation,
};
use crate::task::planning::{PlanningInstructionSelection, PlanningSource};
use crate::task::source::TaskAcceptance;

/// Exact identity without a version alias or an early PublicSchema entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscussionSchema {
    #[serde(rename = "work-discussion-session")]
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Pending,
    Confirmed,
    Deferred,
    Blocked,
    NeedsReview,
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscussionAction {
    AddDecision,
    UpdateDecision,
    UpdatePlanning,
    PrepareQuestion,
    AnswerQuestion,
    UpdateContext,
    UpdateContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAuthorization {
    pub requirement_id: String,
    pub project_root: String,
    pub directory: String,
    pub allowed_actions: Vec<DiscussionAction>,
    pub evidence: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub revoked_reason: Nullable<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementContext {
    pub original_references: Vec<String>,
    pub goal: String,
    pub scope: Vec<String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<TaskAcceptance>,
    /// An absent confirmed context remains absent until the user confirms it.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub confirmed_source: Nullable<PlanningSource>,
    pub work_type: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionResolution {
    pub option_id: String,
    pub rationale: String,
    pub confirmation_evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionDecision {
    pub id: String,
    pub question: String,
    pub question_version: u64,
    pub options: Vec<DecisionOption>,
    pub status: DecisionStatus,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub tentative_option_id: Nullable<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub resolution: Nullable<DecisionResolution>,
    pub status_reason: String,
    pub status_evidence: String,
    pub added_reason: String,
    pub task_ids: Vec<String>,
    pub dependencies: Vec<String>,
    pub source_references: Vec<String>,
}

/// Confirmed domain facts are stored directly, rather than a nested candidate DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionTask {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub scope: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub dependencies: Vec<String>,
    pub decision_ids: Vec<String>,
    pub acceptance_ids: Vec<String>,
    pub acceptance_criteria: Vec<TaskAcceptance>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub instruction_selection: Nullable<PlanningInstructionSelection>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub instructions_sha256: Nullable<String>,
    pub inputs: Vec<CandidateInput>,
    pub files: Vec<CandidateFile>,
    pub risks: Vec<CandidateRisk>,
    pub steps: Vec<CandidateStep>,
    pub commands: Vec<CandidateCommand>,
    pub operations: Vec<CandidateOperation>,
    pub validations: Vec<CandidateValidation>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub review: Nullable<PlanningReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningReview {
    pub context_revision: u64,
    pub decision_versions: BTreeMap<String, u64>,
    pub semantic_consistency_evidence: String,
    pub needs_review: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedQuestion {
    pub decision_id: String,
    pub question_version: u64,
    pub text: String,
    pub display_options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Continuation {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub current_task_id: Nullable<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub question: Nullable<SavedQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionCommit {
    pub operation_id: String,
    pub operation_sha256: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub previous_sha256: Nullable<String>,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionSession {
    pub schema: DiscussionSchema,
    pub requirement_id: String,
    pub revision: u64,
    pub context: RequirementContext,
    pub authorization: SaveAuthorization,
    pub tasks: Vec<DiscussionTask>,
    pub retired_task_ids: Vec<String>,
    pub decisions: Vec<DiscussionDecision>,
    /// Monotonic allocators also retain withdrawn identities.
    pub next_task_number: u64,
    pub next_decision_number: u64,
    pub continuation: Continuation,
    pub commit: DiscussionCommit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn example() -> Value {
        json!({
            "schema":"work-discussion-session", "requirement_id":"example", "revision":1,
            "context":{"original_references":["source.txt"],"goal":"Deliver", "scope":["Task"],
                "constraints":[],"acceptance_criteria":[{"id":"ACCEPTANCE-001","criterion":"Verified"}],
                "confirmed_source":null,"work_type":"task","revision":1},
            "authorization":{"requirement_id":"example","project_root":"/project",
                "directory":"outputs/work/discussions/example", "allowed_actions":["add_decision"],
                "evidence":"User approved scope","revoked_reason":null},
            "tasks":[],"retired_task_ids":[],"decisions":[{
                "id":"D001","question":"Format?","question_version":1,
                "options":[{"id":"csv","label":"CSV","explanation":"Portable"}],
                "status":"pending","tentative_option_id":"csv","resolution":null,
                "status_reason":"","status_evidence":"","added_reason":"Choose format",
                "task_ids":[],"dependencies":[],"source_references":["source.txt"]}],
            "next_task_number":1,"next_decision_number":2,
            "continuation":{"current_task_id":null,"question":{"decision_id":"D001",
                "question_version":1,"text":"Format?","display_options":{"1":"csv"}}},
            "commit":{"operation_id":"init-1","operation_sha256":"a".repeat(64),
                "previous_sha256":null,"content_sha256":"b".repeat(64)}
        })
    }

    #[test]
    fn session_round_trip_keeps_question_mapping_and_unconfirmed_suggestion() {
        let value = example();
        let session: DiscussionSession = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(session.decisions[0].status, DecisionStatus::Pending);
        assert_eq!(session.decisions[0].resolution, Nullable::Null);
        assert_eq!(serde_json::to_value(session).unwrap(), value);
    }

    #[test]
    fn session_rejects_missing_nullable_unknown_nested_fields_and_schema_aliases() {
        for pointer in [
            "/context/confirmed_source",
            "/authorization/revoked_reason",
            "/continuation/current_task_id",
            "/continuation/question",
            "/commit/previous_sha256",
            "/decisions/0/resolution",
            "/decisions/0/tentative_option_id",
        ] {
            let mut value = example();
            let (parent, field) = pointer.rsplit_once('/').unwrap();
            value
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                serde_json::from_value::<DiscussionSession>(value).is_err(),
                "{pointer}"
            );
        }
        for pointer in [
            "",
            "/context",
            "/authorization",
            "/continuation",
            "/continuation/question",
            "/commit",
            "/decisions/0",
            "/decisions/0/options/0",
        ] {
            let mut value = example();
            value.pointer_mut(pointer).unwrap()["unknown"] = json!(true);
            assert!(
                serde_json::from_value::<DiscussionSession>(value).is_err(),
                "{pointer}"
            );
        }
        for schema in [
            "work-discussion-session-v1",
            "work-task-draft",
            "work-discussion-progress",
        ] {
            let mut value = example();
            value["schema"] = json!(schema);
            assert!(serde_json::from_value::<DiscussionSession>(value).is_err());
        }
    }

    #[test]
    fn all_statuses_are_exact_and_invalid_status_is_rejected() {
        for status in [
            "pending",
            "confirmed",
            "deferred",
            "blocked",
            "needs_review",
            "withdrawn",
        ] {
            let typed: DecisionStatus = serde_json::from_value(json!(status)).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), json!(status));
        }
        assert!(serde_json::from_value::<DecisionStatus>(json!("complete")).is_err());
        let mut value = example();
        value["task_candidate"] = json!({});
        assert!(serde_json::from_value::<DiscussionSession>(value).is_err());
    }
}
